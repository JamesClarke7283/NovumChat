//! Toasts, confirmation dialogs, the image lightbox and the unlock screen.

use leptos::prelude::*;
use leptos::task::spawn_local;
use novumchat_core::types::ToastLevel;

use crate::api;
use crate::icons::{Icon, Mark};
use crate::state::use_state;

#[component]
pub fn Toasts() -> impl IntoView {
    let state = use_state();
    view! {
        <div class="nc-toasts" aria-live="polite">
            <For
                each=move || state.toasts.get()
                key=|t| t.id
                children=move |t| {
                    let (class, icon) = match t.level {
                        ToastLevel::Success => ("success", "check-circle"),
                        ToastLevel::Error => ("error", "x-circle"),
                        ToastLevel::Warning => ("warning", "alert"),
                        ToastLevel::Info => ("info", "info"),
                    };
                    let id = t.id;
                    view! {
                        <div class=format!("nc-toast {class}") role="status">
                            <span class="ico"><Icon name=icon size=18/></span>
                            <span style="flex:1">{t.message.clone()}</span>
                            <button aria-label="Dismiss" on:click=move |_| state.toasts.update(|l| l.retain(|x| x.id != id))>
                                <Icon name="x" size=14/>
                            </button>
                        </div>
                    }
                }
            />
        </div>
    }
}

#[component]
pub fn ConfirmHost() -> impl IntoView {
    let state = use_state();
    view! {
        {move || state.confirm.get().map(|c| {
            let on_confirm = c.on_confirm;
            view! {
                <div class="nc-backdrop" on:click=move |_| state.confirm.set(None)>
                    <div class="nc-dialog" role="alertdialog" aria-modal="true" on:click=|ev| ev.stop_propagation()>
                        <h3>{c.title.clone()}</h3>
                        <p>{c.message.clone()}</p>
                        <div class="nc-dialog-actions">
                            <button class="nc-btn nc-btn-ghost" on:click=move |_| state.confirm.set(None)>"Cancel"</button>
                            <button
                                class=if c.danger { "nc-btn nc-btn-danger" } else { "nc-btn nc-btn-primary" }
                                on:click=move |_| {
                                    state.confirm.set(None);
                                    on_confirm.run(());
                                }
                                autofocus=true
                            >
                                {c.confirm.clone()}
                            </button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

#[component]
pub fn Lightbox() -> impl IntoView {
    let state = use_state();
    view! {
        {move || state.lightbox.get().map(|src| view! {
            <div class="nc-lightbox" on:click=move |_| state.lightbox.set(None)>
                <img src=src alt="Attachment"/>
            </div>
        })}
    }
}

#[component]
pub fn UnlockScreen() -> impl IntoView {
    let state = use_state();
    let pass = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let submit = move || {
        if busy.get_untracked() {
            return;
        }
        let p = pass.get_untracked();
        if p.is_empty() {
            return;
        }
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match api::vault_unlock(p).await {
                Ok(status) => {
                    pass.set(String::new());
                    state.vault.set(status);
                    state.load_all().await;
                }
                Err(e) => error.set(Some(e)),
            }
            busy.set(false);
        });
    };
    view! {
        <div class="nc-center-screen">
            <div class="nc-hero-glow" style="inset:8% -10% auto;height:60%"></div>
            <form class="nc-unlock" on:submit=move |ev| {
                ev.prevent_default();
                submit();
            }>
                <Mark size=52 animate=true/>
                <h1>"Your chats are encrypted"</h1>
                <p>"Enter your passphrase to unlock this device's chat history. It never leaves this device."</p>
                <input
                    class="nc-input"
                    type="password"
                    autocomplete="current-password"
                    placeholder="Passphrase"
                    aria-label="Passphrase"
                    prop:value=move || pass.get()
                    on:input=move |ev| pass.set(event_target_value(&ev))
                    autofocus=true
                />
                {move || error.get().map(|e| view! { <div class="nc-form-error" role="alert">{e}</div> })}
                <button class="nc-btn nc-btn-primary" type="submit" disabled=move || busy.get()>
                    {move || if busy.get() { view! { <span class="nc-spinner"></span>"Unlocking…" }.into_any() } else { view! { <Icon name="unlock" size=17/>"Unlock" }.into_any() }}
                </button>
            </form>
        </div>
    }
}
