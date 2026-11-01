//! The conversation column and the empty-chat hero.

use leptos::html;
use leptos::prelude::*;

use crate::components::ensemble::Ensemble;
use crate::components::message::MessageView;
use crate::icons::{Icon, Mark};
use crate::state::{Row, SettingsTab, use_state};

struct Starter {
    icon: &'static str,
    hue: &'static str,
    title: &'static str,
    detail: &'static str,
    prompt: &'static str,
    web: bool,
    tools: bool,
}

const STARTERS: [Starter; 4] = [
    Starter {
        icon: "sparkles",
        hue: "var(--s1)",
        title: "Build a small interactive app",
        detail: "Runs live in the chat, sandboxed",
        prompt: "Build a single-file HTML app: a pomodoro timer with a circular progress ring, start/pause/reset, and a gentle chime. Make it look polished.",
        web: false,
        tools: false,
    },
    Starter {
        icon: "hash",
        hue: "var(--s5)",
        title: "Explain it with equations",
        detail: "LaTeX renders inline and in display mode",
        prompt: "Explain how gradient descent works, with the key equations in LaTeX and a short worked example.",
        web: false,
        tools: false,
    },
    Starter {
        icon: "globe",
        hue: "var(--s2)",
        title: "Research something current",
        detail: "Searches the web and cites sources",
        prompt: "Search the web for the latest stable Rust release and summarise what's new, with sources.",
        web: true,
        tools: false,
    },
    Starter {
        icon: "folder",
        hue: "var(--s3)",
        title: "Work with my files",
        detail: "Reads your workspace; asks before changing anything",
        prompt: "List the files in my workspace and give me a quick overview of what's there.",
        web: false,
        tools: true,
    },
];

#[component]
fn Hero() -> impl IntoView {
    let state = use_state();
    let models =
        Memo::new(move |_| state.models.with(|m| (m.models.len(), m.providers.iter().filter(|p| p.ok).count())));
    let selected = Memo::new(move |_| state.settings_now().models.len());
    let animate = !crate::bridge::load("novum-seen-hero").is_some_and(|v| v == "1");
    crate::bridge::store("novum-seen-hero", "1");

    let use_starter = move |s: &'static Starter| {
        state.update_settings(|set| {
            if s.web {
                set.web_search = true;
            }
            if s.tools {
                set.tools = true;
            }
        });
        state.draft_text.set(s.prompt.to_string());
        state.focus_composer.update(|v| *v += 1);
    };

    view! {
        <section class="nc-hero">
            <div class="nc-hero-glow"></div>
            <Mark size=56 animate=animate/>
            <h1>"Ask once. Hear from every model."</h1>
            <p class="nc-hero-sub">
                {move || {
                    let (n, p) = models.get();
                    let sel = selected.get();
                    if state.models_loading.get() && n == 0 {
                        "Looking for models on your providers…".to_string()
                    } else if n == 0 {
                        "No models are reachable yet. Start Ollama or add an OpenAI-compatible endpoint to begin.".to_string()
                    } else if sel > 1 {
                        format!("{sel} models will answer side by side. {n} models are available across {p} provider{}.", if p == 1 { "" } else { "s" })
                    } else {
                        format!("{n} model{} available across {p} provider{}. Pick several in the model menu to compare their answers.", if n == 1 { "" } else { "s" }, if p == 1 { "" } else { "s" })
                    }
                }}
            </p>
            <Show when=move || models.get().0 == 0 && !state.models_loading.get()>
                <div style="margin-top:20px;display:flex;gap:8px;flex-wrap:wrap">
                    <button class="nc-btn nc-btn-primary" on:click=move |_| state.settings.set(Some(SettingsTab::Providers))>
                        <Icon name="plus" size=16/>
                        "Add a provider"
                    </button>
                    <button class="nc-btn nc-btn-secondary" on:click=move |_| {
                        leptos::task::spawn_local(async move { state.refresh_models(true).await });
                    }>
                        <Icon name="refresh" size=16/>
                        "Check again"
                    </button>
                </div>
            </Show>
            <div class="nc-starters">
                {STARTERS
                    .iter()
                    .map(|s| view! {
                        <button class="nc-starter" style=format!("--hue:{}", s.hue) on:click=move |_| use_starter(s)>
                            <span class="nc-starter-icon"><Icon name=s.icon size=18/></span>
                            <span>
                                <b>{s.title}</b>
                                <span>{s.detail}</span>
                            </span>
                        </button>
                    })
                    .collect_view()}
            </div>
        </section>
    }
}

#[component]
pub fn Thread() -> impl IntoView {
    let state = use_state();
    let scroller: NodeRef<html::Div> = NodeRef::new();
    let at_bottom = RwSignal::new(true);
    let rows = Memo::new(move |_| state.rows());
    let empty = Memo::new(move |_| rows.with(|r| r.is_empty()) && !state.loading_chat.get());
    let wide = Memo::new(move |_| rows.with(|r| r.iter().any(|r| matches!(r, Row::Group { .. }))));

    let scroll_to_bottom = move |smooth: bool| {
        if let Some(el) = scroller.get_untracked() {
            let opts = web_sys::ScrollToOptions::new();
            opts.set_top(el.scroll_height() as f64);
            opts.set_behavior(if smooth { web_sys::ScrollBehavior::Smooth } else { web_sys::ScrollBehavior::Instant });
            el.scroll_to_with_scroll_to_options(&opts);
        }
    };

    // Keep the newest text in view while streaming, unless the user scrolled up.
    Effect::new(move |_| {
        rows.track();
        state.tick.track();
        if at_bottom.get_untracked() {
            request_animation_frame(move || scroll_to_bottom(false));
        }
    });
    // Jump to the end when a chat is opened.
    Effect::new(move |_| {
        state.current.track();
        if !state.loading_chat.get() {
            at_bottom.set(true);
            request_animation_frame(move || scroll_to_bottom(false));
        }
    });

    let on_scroll = move |_| {
        if let Some(el) = scroller.get_untracked() {
            let gap = el.scroll_height() - el.scroll_top() - el.client_height();
            at_bottom.set(gap < 90);
        }
    };

    view! {
        <div class="nc-scroll" node_ref=scroller on:scroll=on_scroll>
            <div class="nc-thread" class:is-wide=move || wide.get()>
                <Show when=move || empty.get()>
                    <Hero/>
                </Show>
                <For
                    each=move || rows.get()
                    key=|row| match row {
                        Row::Msg(id) => id.clone(),
                        Row::Group { group, members, active } => format!("g:{group}:{active}:{}", members.len()),
                    }
                    children=move |row| match row {
                        Row::Msg(id) => view! { <MessageView id=id/> }.into_any(),
                        Row::Group { group, members, active } => view! { <Ensemble group=group members=members active=active/> }.into_any(),
                    }
                />
            </div>
        </div>
        <Show when=move || !at_bottom.get() && !empty.get()>
            <button class="nc-jump" on:click=move |_| {
                at_bottom.set(true);
                scroll_to_bottom(true);
            }>
                <Icon name="arrow-down" size=15/>
                "Jump to latest"
            </button>
        </Show>
    }
}
