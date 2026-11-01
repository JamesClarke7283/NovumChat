//! Message input: text, attachments (picker, drag & drop, paste), web/tools
//! toggles, model selection and send/stop.

use leptos::html;
use leptos::prelude::*;
use novumchat_core::permissions::PermissionLevel;
use novumchat_core::util::human_bytes;
use wasm_bindgen::JsCast;

use crate::bridge;
use crate::components::model_picker::{ModelPicker, PermissionMenu};
use crate::icons::Icon;
use crate::state::{AppState, PendingAttachment, use_state};
use crate::util::{coarse_pointer, file_icon, hue_for, short_model};

const MAX_FILES: usize = 12;

pub fn add_files(state: AppState, files: web_sys::FileList) {
    let mut added = Vec::new();
    for i in 0..files.length() {
        let Some(f) = files.get(i) else { continue };
        let mime = f.type_();
        let preview = mime.starts_with("image/").then(|| bridge::object_url(&f));
        added.push(PendingAttachment {
            id: state.next_id(),
            name: f.name(),
            mime,
            size: f.size() as u64,
            preview,
            file: f,
        });
    }
    if added.is_empty() {
        return;
    }
    state.draft_files.update(|list| {
        for a in added {
            if list.len() < MAX_FILES {
                list.push(a);
            }
        }
    });
    state.focus_composer.update(|v| *v += 1);
}

fn has_files(dt: Option<web_sys::DataTransfer>) -> bool {
    dt.map(|d| {
        let types = d.types();
        (0..types.length()).any(|i| types.get(i).as_string().as_deref() == Some("Files"))
    })
    .unwrap_or(false)
}

fn resize(el: &web_sys::HtmlTextAreaElement) {
    let style = web_sys::HtmlElement::style(el);
    let _ = style.set_property("height", "auto");
    let h = el.scroll_height();
    let _ = style.set_property("height", &format!("{}px", h.max(26)));
}

#[component]
pub fn Composer() -> impl IntoView {
    let state = use_state();
    let textarea: NodeRef<html::Textarea> = NodeRef::new();
    let file_input: NodeRef<html::Input> = NodeRef::new();
    let picker_open = RwSignal::new(false);
    let tools_open = RwSignal::new(false);
    let dragging = RwSignal::new(false);
    let busy = Memo::new(move |_| state.busy());
    let settings = Memo::new(move |_| state.settings_now());
    let can_send = Memo::new(move |_| {
        (!state.draft_text.with(|t| t.trim().is_empty()) || !state.draft_files.with(|f| f.is_empty()))
            && !state.sending.get()
    });
    let permission = Memo::new(move |_| state.permission());

    // Focus on request (new chat, starter chosen, files added).
    Effect::new(move |_| {
        state.focus_composer.track();
        if let Some(el) = textarea.get_untracked()
            && !coarse_pointer()
        {
            crate::render::focus_later(el.clone().unchecked_into());
        }
    });
    // Resize when the text changes programmatically.
    Effect::new(move |_| {
        state.draft_text.track();
        if let Some(el) = textarea.get() {
            request_animation_frame(move || resize(&el));
        }
    });

    // Drag & drop anywhere in the window.
    let drag_depth = StoredValue::new(0i32);
    let h1 = window_event_listener(leptos::ev::dragenter, move |ev: web_sys::DragEvent| {
        if has_files(ev.data_transfer()) {
            ev.prevent_default();
            drag_depth.update_value(|d| *d += 1);
            dragging.set(true);
        }
    });
    let h2 = window_event_listener(leptos::ev::dragover, move |ev: web_sys::DragEvent| {
        if has_files(ev.data_transfer()) {
            ev.prevent_default();
        }
    });
    let h3 = window_event_listener(leptos::ev::dragleave, move |_ev: web_sys::DragEvent| {
        drag_depth.update_value(|d| *d -= 1);
        if drag_depth.get_value() <= 0 {
            drag_depth.set_value(0);
            dragging.set(false);
        }
    });
    let h4 = window_event_listener(leptos::ev::drop, move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        drag_depth.set_value(0);
        dragging.set(false);
        if let Some(files) = ev.data_transfer().and_then(|d| d.files()) {
            add_files(state, files);
        }
    });
    on_cleanup(move || {
        h1.remove();
        h2.remove();
        h3.remove();
        h4.remove();
    });

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() != "Enter" || ev.is_composing() {
            return;
        }
        let send_on_enter = state.config.with_untracked(|c| c.as_ref().is_none_or(|c| c.general.send_on_enter));
        let wants_send =
            if send_on_enter && !coarse_pointer() { !ev.shift_key() } else { ev.ctrl_key() || ev.meta_key() };
        if wants_send {
            ev.prevent_default();
            if !busy.get_untracked() {
                state.send();
            }
        }
    };
    let on_paste = move |ev: web_sys::ClipboardEvent| {
        if let Some(files) = ev.clipboard_data().and_then(|d| d.files())
            && files.length() > 0
        {
            ev.prevent_default();
            add_files(state, files);
        }
    };

    let model_label = move || {
        let models = settings.get().models;
        match models.len() {
            0 => view! { <Icon name="cpu" size=16/><span class="lbl">"Choose model"</span> }.into_any(),
            1 => {
                let m = models[0].clone();
                view! {
                    <span class="nc-hue-dot" style=format!("--hue:{}", hue_for(&m))></span>
                    <span class="lbl nc-truncate">{short_model(&m)}</span>
                    <Icon name="chevron-down" size=14/>
                }
                .into_any()
            }
            n => view! {
                <span class="nc-hues">
                    {models.iter().take(5).map(|m| view! { <span class="nc-hue-dot" style=format!("--hue:{}", hue_for(m))></span> }).collect_view()}
                </span>
                <span class="lbl">{format!("{n} models")}</span>
                <Icon name="chevron-down" size=14/>
            }
            .into_any(),
        }
    };

    view! {
        <Show when=move || dragging.get()>
            <div class="nc-drop">
                <div>
                    <Icon name="paperclip" size=34/>
                    "Drop files to attach"
                    <small style="font-weight:500;color:var(--muted);font-size:13px">"Images, documents, PDFs and code"</small>
                </div>
            </div>
        </Show>
        <div class="nc-composer-dock">
            <div class="nc-composer" class:is-live=move || busy.get()>
                <Show when=move || !state.draft_files.with(|f| f.is_empty())>
                    <div class="nc-attachments">
                        <For
                            each=move || state.draft_files.get()
                            key=|a| a.id
                            children=move |a| {
                                let id = a.id;
                                let icon = file_icon(&a.name, &a.mime);
                                view! {
                                    <div class="nc-att" title=a.name.clone()>
                                        {match a.preview.clone() {
                                            Some(url) => view! { <img src=url alt=""/> }.into_any(),
                                            None => view! { <span class="ico"><Icon name=icon size=20/></span> }.into_any(),
                                        }}
                                        <span style="min-width:0">
                                            <b>{a.name.clone()}</b>
                                            <small>{human_bytes(a.size)}</small>
                                        </span>
                                        <button aria-label="Remove attachment" on:click=move |_| {
                                            state.draft_files.update(|l| {
                                                if let Some(p) = l.iter().position(|x| x.id == id) {
                                                    let removed = l.remove(p);
                                                    if let Some(u) = removed.preview {
                                                        bridge::revoke_url(&u);
                                                    }
                                                }
                                            });
                                        }>
                                            <Icon name="x" size=12/>
                                        </button>
                                    </div>
                                }
                            }
                        />
                    </div>
                </Show>
                <textarea
                    node_ref=textarea
                    rows="1"
                    aria-label="Message"
                    placeholder={move || if settings.get().models.len() > 1 { "Ask all selected models…" } else { "Message NovumChat…" }}
                    prop:value=move || state.draft_text.get()
                    on:input=move |ev| {
                        state.draft_text.set(event_target_value(&ev));
                        if let Some(el) = textarea.get_untracked() {
                            resize(&el);
                        }
                    }
                    on:keydown=on_keydown
                    on:paste=on_paste
                ></textarea>
                <div class="nc-composer-bar">
                    <input
                        node_ref=file_input
                        type="file"
                        multiple=true
                        style="display:none"
                        on:change=move |_| {
                            if let Some(input) = file_input.get_untracked() {
                                if let Some(files) = input.files() {
                                    add_files(state, files);
                                }
                                input.set_value("");
                            }
                        }
                    />
                    <button class="nc-chip-btn" title="Attach files" aria-label="Attach files" on:click=move |_| {
                        if let Some(i) = file_input.get_untracked() {
                            i.click();
                        }
                    }>
                        <Icon name="paperclip" size=17/>
                    </button>
                    <button
                        class="nc-chip-btn"
                        class:is-on=move || settings.get().web_search
                        title="Let models search the web"
                        aria-pressed=move || settings.get().web_search.to_string()
                        on:click=move |_| state.update_settings(|s| s.web_search = !s.web_search)
                    >
                        <Icon name="globe" size=17/>
                        <span class="lbl">"Web"</span>
                    </button>
                    <span style="position:relative;display:inline-flex">
                        <button
                            class="nc-chip-btn"
                            class:is-on=move || settings.get().tools && permission.get() != PermissionLevel::Yolo
                            class:is-yolo=move || settings.get().tools && permission.get() == PermissionLevel::Yolo
                            title="Tools and permissions"
                            on:click=move |_| tools_open.update(|o| *o = !*o)
                        >
                            <Icon name="tool" size=17/>
                            <span class="lbl">
                                {move || if !settings.get().tools {
                                    "Tools off"
                                } else {
                                    match permission.get() {
                                        PermissionLevel::ReadOnly => "Read only",
                                        PermissionLevel::Ask => "Ask first",
                                        PermissionLevel::AutoEdit => "Auto-edit",
                                        PermissionLevel::Yolo => "YOLO",
                                    }
                                }}
                            </span>
                        </button>
                        <Show when=move || tools_open.get()>
                            <PermissionMenu open=tools_open/>
                        </Show>
                    </span>
                    <span style="position:relative;display:inline-flex;min-width:0">
                        <button class="nc-chip-btn models" title="Choose models" on:click=move |_| picker_open.update(|o| *o = !*o)>
                            {model_label}
                        </button>
                        <Show when=move || picker_open.get()>
                            <ModelPicker open=picker_open/>
                        </Show>
                    </span>
                    <button
                        class="nc-send"
                        class:is-stop=move || busy.get()
                        aria-label={move || if busy.get() { "Stop generating" } else { "Send" }}
                        title={move || if busy.get() { "Stop (Esc)" } else { "Send (Enter)" }}
                        disabled=move || !busy.get() && !can_send.get()
                        on:click=move |_| {
                            if busy.get_untracked() {
                                state.stop();
                            } else {
                                state.send();
                            }
                        }
                    >
                        {move || if busy.get() {
                            view! { <Icon name="stop" size=16/> }.into_any()
                        } else if state.sending.get() {
                            view! { <span class="nc-spinner"></span> }.into_any()
                        } else {
                            view! { <Icon name="send" size=19/> }.into_any()
                        }}
                    </button>
                </div>
            </div>
            <div class="nc-composer-hint">
                {move || if state.config.with(|c| c.as_ref().is_none_or(|c| c.general.send_on_enter)) {
                    "Enter to send, Shift + Enter for a new line. Answers can be wrong; check what matters."
                } else {
                    "Ctrl + Enter to send. Answers can be wrong; check what matters."
                }}
            </div>
        </div>
    }
}
