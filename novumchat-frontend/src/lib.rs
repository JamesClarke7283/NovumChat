//! NovumChat user interface (Leptos, client-side rendered WebAssembly).

pub mod api;
pub mod bridge;
pub mod components;
pub mod icons;
pub mod render;
pub mod state;
pub mod util;

use leptos::prelude::*;
use novumchat_core::config::ThemeMode;
use wasm_bindgen::JsCast;

use crate::components::composer::Composer;
use crate::components::header::Header;
use crate::components::overlays::{ConfirmHost, Lightbox, Toasts, UnlockScreen};
use crate::components::settings::SettingsModal;
use crate::components::sidebar::Sidebar;
use crate::components::thread::Thread;
use crate::icons::Mark;
use crate::render::FullscreenArtifact;
use crate::state::{AppState, Boot, use_state};

/// Applies theme, accent, text size and motion preferences to `<html>`.
fn install_theme(state: AppState) {
    Effect::new(move |_| {
        let system_dark = state.system_dark.get();
        let (mode, accent, scale, animations) = state.config.with(|c| match c {
            Some(c) => {
                (c.appearance.theme, c.appearance.accent.clone(), c.appearance.font_scale, c.appearance.animations)
            }
            None => {
                let mode = match bridge::load("novum-theme").as_deref() {
                    Some("light") => ThemeMode::Light,
                    Some("dark") => ThemeMode::Dark,
                    _ => ThemeMode::Auto,
                };
                (mode, bridge::load("novum-accent").unwrap_or_else(|| "aurora".into()), 1.0, true)
            }
        });
        let dark = match mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::Auto => system_dark,
        };
        state.dark.set(dark);
        if let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) {
            let _ = root.set_attribute("data-theme", if dark { "dark" } else { "light" });
            let _ = root.set_attribute("data-accent", &accent);
            let _ = root.set_attribute("data-motion", if animations { "on" } else { "off" });
            if let Some(el) = root.dyn_ref::<web_sys::HtmlElement>() {
                let _ = el.style().set_property("--font-scale", &format!("{scale:.2}"));
            }
        }
        bridge::store("novum-theme", mode.as_str());
        bridge::store("novum-accent", &accent);
    });
}

fn install_shortcuts(state: AppState) {
    let handle = window_event_listener(leptos::ev::keydown, move |ev: web_sys::KeyboardEvent| {
        let ctrl = ev.ctrl_key() || ev.meta_key();
        let key = ev.key();
        if ctrl && ev.shift_key() && (key == "O" || key == "o") {
            ev.prevent_default();
            state.new_chat();
        } else if ctrl && (key == "k" || key == "K") {
            ev.prevent_default();
            if let Some(el) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.query_selector(".nc-search input").ok().flatten())
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                state.sidebar_hidden.set(false);
                state.drawer.set(true);
                let _ = el.focus();
            }
        } else if ctrl && key == "," {
            ev.prevent_default();
            state.settings.set(Some(state::SettingsTab::General));
        } else if key == "Escape" {
            if state.fullscreen.with_untracked(|f| f.is_some()) {
                state.fullscreen.set(None);
            } else if state.lightbox.with_untracked(|l| l.is_some()) {
                state.lightbox.set(None);
            } else if state.confirm.with_untracked(|c| c.is_some()) {
                state.confirm.set(None);
            } else if state.settings.with_untracked(|s| s.is_some()) {
                state.settings.set(None);
            } else if state.drawer.get_untracked() {
                state.drawer.set(false);
            } else if state.busy() {
                state.stop();
            }
        }
    });
    // Lives for the whole app.
    std::mem::forget(handle);
}

#[component]
pub fn App() -> impl IntoView {
    let state = AppState::new();
    provide_context(state);
    install_theme(state);
    install_shortcuts(state);
    render::install_runner_resizer();
    state.start();

    let boot = Memo::new(move |_| state.boot.get());
    let unlocked = Memo::new(move |_| state.vault.with(|v| v.unlocked));

    view! {
        {move || match boot.get() {
            Boot::Loading => view! { <BootScreen/> }.into_any(),
            Boot::Failed(err) => view! { <FailedScreen error=err/> }.into_any(),
            Boot::Ready => {
                if unlocked.get() {
                    view! { <Shell/> }.into_any()
                } else {
                    view! { <UnlockScreen/> }.into_any()
                }
            }
        }}
        <SettingsModal/>
        <ConfirmHost/>
        <FullscreenArtifact/>
        <Lightbox/>
        <Toasts/>
    }
}

#[component]
fn BootScreen() -> impl IntoView {
    view! {
        <div class="nc-center-screen">
            <div class="nc-hero-glow" style="inset:20% 10% auto;height:50%"></div>
            <Mark size=64 animate=true/>
        </div>
    }
}

#[component]
fn FailedScreen(error: String) -> impl IntoView {
    view! {
        <div class="nc-center-screen">
            <div class="nc-unlock">
                <Mark size=48/>
                <h1>"NovumChat couldn't start"</h1>
                <p>"The backend reported an error while opening your settings or chat database:"</p>
                <div class="nc-error"><span>{error}</span></div>
                <p style="font-size:13.5px;color:var(--muted)">
                    "Check that the data directory is writable and that config.toml is valid, then restart the app."
                </p>
                <button class="nc-btn nc-btn-primary" on:click=move |_| {
                    if let Some(w) = web_sys::window() {
                        let _ = w.location().reload();
                    }
                }>"Try again"</button>
            </div>
        </div>
    }
}

#[component]
fn Shell() -> impl IntoView {
    let state = use_state();
    // Edge swipe opens the drawer on touch screens.
    let touch_start = StoredValue::new(None::<(f64, f64)>);
    let on_touch_start = move |ev: web_sys::TouchEvent| {
        if let Some(t) = ev.touches().get(0) {
            let x = t.client_x() as f64;
            let y = t.client_y() as f64;
            let drawer = state.drawer.get_untracked();
            touch_start.set_value(if x < 28.0 || drawer { Some((x, y)) } else { None });
        }
    };
    let on_touch_move = move |ev: web_sys::TouchEvent| {
        let Some((x0, y0)) = touch_start.get_value() else { return };
        let Some(t) = ev.touches().get(0) else { return };
        let dx = t.client_x() as f64 - x0;
        let dy = (t.client_y() as f64 - y0).abs();
        if dy > 40.0 {
            touch_start.set_value(None);
            return;
        }
        if !state.drawer.get_untracked() && dx > 60.0 && util::is_narrow() {
            state.drawer.set(true);
            touch_start.set_value(None);
        } else if state.drawer.get_untracked() && dx < -60.0 {
            state.drawer.set(false);
            touch_start.set_value(None);
        }
    };

    view! {
        <div
            class="nc-app"
            class:drawer-open=move || state.drawer.get()
            class:sidebar-hidden=move || state.sidebar_hidden.get()
            on:touchstart=on_touch_start
            on:touchmove=on_touch_move
        >
            <div class="nc-scrim" on:click=move |_| state.drawer.set(false)></div>
            <Sidebar/>
            <main class="nc-main">
                <Header/>
                <Thread/>
                <Composer/>
            </main>
        </div>
    }
}
