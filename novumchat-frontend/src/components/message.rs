//! User and assistant messages.

use std::cell::RefCell;
use std::collections::HashMap;

use leptos::prelude::*;
use leptos::task::spawn_local;
use novumchat_core::types::*;
use novumchat_core::util::format_duration_ms;
use novumchat_core::util::tree::Tree;
use wasm_bindgen::prelude::*;

use crate::components::tool::ToolCallView;
use crate::icons::Icon;
use crate::render::Markdown;
use crate::state::use_state;
use crate::util::{file_icon, hue_style, short_model};
use crate::{api, bridge};

thread_local! {
    static ATTACHMENT_URLS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Object URL for an attachment (fetched once, then cached).
fn attachment_url(meta: AttachmentMeta, out: RwSignal<Option<String>>) {
    if let Some(url) = ATTACHMENT_URLS.with(|m| m.borrow().get(&meta.id).cloned()) {
        out.set(Some(url));
        return;
    }
    spawn_local(async move {
        if let Ok(data) = api::get_attachment(&meta.id).await {
            let url = bridge::b64_to_object_url(&data.data, &meta.mime);
            ATTACHMENT_URLS.with(|m| m.borrow_mut().insert(meta.id.clone(), url.clone()));
            out.set(Some(url));
        }
    });
}

#[component]
pub fn MessageView(id: Id) -> impl IntoView {
    let state = use_state();
    let Some(sig) = state.message(&id) else {
        return ().into_any();
    };
    let role = sig.with_untracked(|m| m.role);
    match role {
        Role::User => view! { <UserMessage sig=sig/> }.into_any(),
        _ => view! { <AssistantMessage sig=sig in_group=false/> }.into_any(),
    }
}

/// `‹ 2 / 3 ›` navigation between alternative messages.
#[component]
pub fn BranchSwitcher(id: Id) -> impl IntoView {
    let state = use_state();
    let info = Memo::new(move |_| {
        state.structure.track();
        let msgs = state.snapshot();
        let tree = Tree::new(&msgs);
        let m = tree.get(&id)?;
        let (units, idx) = tree.sibling_units(m);
        if units.len() < 2 {
            return None;
        }
        let pick = |i: usize| units.get(i).and_then(|u| u.first()).map(|m| m.id.clone());
        Some((idx, units.len(), idx.checked_sub(1).and_then(pick), pick(idx + 1)))
    });
    view! {
        {move || info.get().map(|(idx, n, prev, next)| {
            let prev_c = prev.clone();
            let next_c = next.clone();
            view! {
                <span class="nc-branch" title="Alternative versions">
                    <button class="nc-icon-btn" aria-label="Previous version" disabled=prev.is_none() on:click=move |_| {
                        if let Some(p) = prev_c.clone() {
                            state.set_leaf(p);
                        }
                    }>
                        <Icon name="chevron-left" size=15/>
                    </button>
                    {format!("{} / {}", idx + 1, n)}
                    <button class="nc-icon-btn" aria-label="Next version" disabled=next.is_none() on:click=move |_| {
                        if let Some(p) = next_c.clone() {
                            state.set_leaf(p);
                        }
                    }>
                        <Icon name="chevron-right" size=15/>
                    </button>
                </span>
            }
        })}
    }
}

#[component]
fn CopyButton(#[prop(into)] text: Signal<String>) -> impl IntoView {
    let done = RwSignal::new(false);
    view! {
        <button class="nc-icon-btn sm" title="Copy" aria-label="Copy" on:click=move |_| {
            let t = text.get_untracked();
            spawn_local(async move {
                if bridge::copy(&t).await {
                    done.set(true);
                    set_timeout(move || done.set(false), std::time::Duration::from_millis(1300));
                }
            });
        }>
            {move || if done.get() { view! { <Icon name="check" size=16/> }.into_any() } else { view! { <Icon name="copy" size=16/> }.into_any() }}
        </button>
    }
}

#[component]
fn Attachments(list: Vec<AttachmentMeta>) -> impl IntoView {
    let state = use_state();
    view! {
        <div class="nc-user-files">
            {list
                .into_iter()
                .map(|a| {
                    if a.kind == AttachmentKind::Image {
                        let url = RwSignal::new(None::<String>);
                        attachment_url(a.clone(), url);
                        let name = a.name.clone();
                        view! {
                            <img
                                class="nc-user-img"
                                alt=name
                                src=move || url.get().unwrap_or_default()
                                on:click=move |_| state.lightbox.set(url.get_untracked())
                            />
                        }
                        .into_any()
                    } else {
                        let icon = file_icon(&a.name, &a.mime);
                        view! {
                            <span class="nc-file-pill" title=a.name.clone()>
                                <span class="ico"><Icon name=icon size=15/></span>
                                <span style="min-width:0">
                                    <span class="nc-truncate" style="display:block;font-weight:600">{a.name.clone()}</span>
                                    <small>{novumchat_core::util::human_bytes(a.size)}</small>
                                </span>
                            </span>
                        }
                        .into_any()
                    }
                })
                .collect_view()}
        </div>
    }
}

#[component]
fn UserMessage(sig: ArcRwSignal<Message>) -> impl IntoView {
    let state = use_state();
    let id = sig.with_untracked(|m| m.id.clone());
    let text = {
        let sig = sig.clone();
        Memo::new(move |_| sig.with(|m| m.text()))
    };
    let attachments = sig.with_untracked(|m| m.attachments.clone());
    let editing = {
        let id = id.clone();
        Memo::new(move |_| state.editing.with(|e| e.as_deref() == Some(id.as_str())))
    };
    let edit_text = RwSignal::new(String::new());
    let id_edit = id.clone();
    let id_send = id.clone();
    let id_branch = id.clone();

    view! {
        <div class="nc-msg nc-msg-user nc-enter">
            {(!attachments.is_empty()).then(|| view! { <Attachments list=attachments.clone()/> })}
            <Show
                when=move || editing.get()
                fallback=move || view! {
                    <Show when=move || !text.with(|t| t.is_empty())>
                        <div class="nc-bubble">{move || text.get()}</div>
                    </Show>
                }
            >
                <div class="nc-edit">
                    <textarea
                        prop:value=move || edit_text.get()
                        on:input=move |ev| edit_text.set(event_target_value(&ev))
                        on:keydown={
                            let id = id_send.clone();
                            move |ev: web_sys::KeyboardEvent| {
                                if ev.key() == "Enter" && (ev.ctrl_key() || ev.meta_key()) {
                                    ev.prevent_default();
                                    state.edit_and_resend(id.clone(), edit_text.get_untracked());
                                } else if ev.key() == "Escape" {
                                    state.editing.set(None);
                                }
                            }
                        }
                        autofocus=true
                    ></textarea>
                    <div style="display:flex;gap:8px">
                        <button class="nc-btn nc-btn-ghost nc-btn-sm" on:click=move |_| state.editing.set(None)>"Cancel"</button>
                        <button class="nc-btn nc-btn-primary nc-btn-sm" on:click={
                            let id = id_send.clone();
                            move |_| state.edit_and_resend(id.clone(), edit_text.get_untracked())
                        }>"Send edited message"</button>
                    </div>
                </div>
            </Show>
            <Show when=move || !editing.get()>
                <div class="nc-msg-foot">
                    <BranchSwitcher id=id_branch.clone()/>
                    <span class="nc-msg-actions">
                        <CopyButton text=Signal::derive(move || text.get())/>
                        <button class="nc-icon-btn sm" title="Edit" aria-label="Edit message" on:click={
                            let id = id_edit.clone();
                            move |_| {
                                edit_text.set(text.get_untracked());
                                state.editing.set(Some(id.clone()));
                            }
                        }>
                            <Icon name="edit" size=16/>
                        </button>
                    </span>
                </div>
            </Show>
        </div>
    }
}

#[component]
fn Reasoning(sig: ArcRwSignal<Message>, index: usize) -> impl IntoView {
    let state = use_state();
    let part = {
        let sig = sig.clone();
        Memo::new(move |_| {
            sig.with(|m| match m.parts.get(index) {
                Some(Part::Reasoning { text, duration_ms }) => (text.clone(), *duration_ms),
                _ => (String::new(), None),
            })
        })
    };
    let thinking = {
        let sig = sig.clone();
        Memo::new(move |_| sig.with(|m| m.is_active() && m.parts.len() == index + 1))
    };
    let expand_default = state.config.with_untracked(|c| c.as_ref().is_some_and(|c| c.appearance.expand_reasoning));
    let open = RwSignal::new(expand_default);
    view! {
        <div class="nc-reasoning" class:is-open=move || open.get()>
            <button class="nc-reasoning-toggle" aria-expanded=move || open.get().to_string() on:click=move |_| open.update(|o| *o = !*o)>
                <Icon name="brain" size=15/>
                {move || {
                    if thinking.get() {
                        view! { <span class="nc-shimmer">"Thinking…"</span> }.into_any()
                    } else {
                        let label = match part.get().1 {
                            Some(ms) => format!("Thought for {}", format_duration_ms(ms)),
                            None => "Reasoning".to_string(),
                        };
                        view! { <span>{label}</span> }.into_any()
                    }
                }}
                <Icon name="chevron-right" size=14 class="chev"/>
            </button>
            <Show when=move || open.get()>
                <div class="nc-reasoning-body">{move || part.get().0}</div>
            </Show>
        </div>
    }
}

#[component]
fn PartView(sig: ArcRwSignal<Message>, index: usize) -> impl IntoView {
    let kind = sig.with_untracked(|m| match m.parts.get(index) {
        Some(Part::Text { .. }) => 0,
        Some(Part::Reasoning { .. }) => 1,
        Some(Part::ToolCall(_)) => 2,
        None => 3,
    });
    match kind {
        0 => {
            let text = {
                let sig = sig.clone();
                Memo::new(move |_| {
                    sig.with(|m| match m.parts.get(index) {
                        Some(Part::Text { text }) => text.clone(),
                        _ => String::new(),
                    })
                })
            };
            let live = {
                let sig = sig.clone();
                Memo::new(move |_| sig.with(|m| m.is_active() && m.parts.len() == index + 1))
            };
            view! {
                <div class:nc-live=move || live.get()>
                    <Markdown text=Signal::derive(move || text.get()) live=Signal::derive(move || live.get())/>
                </div>
            }
            .into_any()
        }
        1 => view! { <Reasoning sig=sig index=index/> }.into_any(),
        2 => view! { <ToolCallView sig=sig index=index/> }.into_any(),
        _ => ().into_any(),
    }
}

/// Body of an assistant message (parts, status, error); shared with ensemble columns.
#[component]
pub fn AssistantBody(sig: ArcRwSignal<Message>) -> impl IntoView {
    let count = {
        let sig = sig.clone();
        Memo::new(move |_| sig.with(|m| m.parts.len()))
    };
    let status = {
        let sig = sig.clone();
        Memo::new(move |_| sig.with(|m| m.status))
    };
    let error = {
        let sig = sig.clone();
        Memo::new(move |_| sig.with(|m| m.error.clone()))
    };
    let sig_parts = sig.clone();
    view! {
        <For
            each=move || 0..count.get()
            key=|i| *i
            children=move |i| view! { <PartView sig=sig_parts.clone() index=i/> }
        />
        <Show when=move || count.get() == 0 && status.get().is_active()>
            <div class="nc-status-line">
                <span class="nc-shimmer">
                    {move || if status.get() == MessageStatus::Pending { "Connecting…" } else { "Thinking…" }}
                </span>
            </div>
        </Show>
        <Show when=move || count.get() == 0 && status.get() == MessageStatus::Cancelled>
            <div class="nc-status-line">"Stopped before any reply."</div>
        </Show>
        {move || error.get().map(|e| view! {
            <div class="nc-error" role="alert">
                <Icon name="alert" size=16/>
                <span>{e}</span>
            </div>
        })}
    }
}

#[component]
fn RegenMenu(id: Id, open: RwSignal<bool>) -> impl IntoView {
    let state = use_state();
    let id = StoredValue::new(id);
    view! {
        <div style="position:fixed;inset:0;z-index:39" on:click=move |_| open.set(false)></div>
        <div class="nc-menu" style="left:0;bottom:calc(100% + 6px);max-height:320px;overflow-y:auto;width:260px">
            {move || state.models.with(|ms| ms.models.iter().map(|m| {
                let r = m.model_ref();
                let label = r.short_label();
                let provider = m.provider_name.clone();
                let style = hue_style(Some(&r));
                view! {
                    <button style=style on:click=move |_| {
                        open.set(false);
                        state.regenerate(id.get_value(), Some(vec![r.clone()]));
                    }>
                        <span class="nc-hue-dot"></span>
                        <span class="nc-truncate" style="flex:1">{label}</span>
                        <small style="color:var(--muted)">{provider}</small>
                    </button>
                }
            }).collect_view())}
        </div>
    }
}

#[component]
pub fn MessageFooter(sig: ArcRwSignal<Message>, #[prop(default = true)] branches: bool) -> impl IntoView {
    let state = use_state();
    let id = sig.with_untracked(|m| m.id.clone());
    let text = {
        let sig = sig.clone();
        Memo::new(move |_| sig.with(|m| m.text()))
    };
    let meta = {
        let sig = sig.clone();
        Memo::new(move |_| sig.with(|m| (m.status, m.duration_ms, m.usage, m.ttft_ms)))
    };
    let speaking = RwSignal::new(false);
    let regen_menu = RwSignal::new(false);
    let can_speak = bridge::can_speak();
    let id_sv = StoredValue::new(id.clone());

    view! {
        <div class="nc-msg-foot">
            {branches.then(|| view! { <BranchSwitcher id=id.clone()/> })}
            <Show when=move || !meta.get().0.is_active()>
                <span class="nc-msg-actions">
                    <CopyButton text=Signal::derive(move || text.get())/>
                    <span style="position:relative;display:inline-flex">
                        <button class="nc-icon-btn sm" title="Regenerate" aria-label="Regenerate" on:click=move |_| state.regenerate(id_sv.get_value(), None)>
                            <Icon name="refresh" size=16/>
                        </button>
                        <button class="nc-icon-btn sm" style="width:20px" title="Regenerate with another model" aria-label="Regenerate with another model" on:click=move |_| regen_menu.update(|m| *m = !*m)>
                            <Icon name="chevron-down" size=13/>
                        </button>
                        <Show when=move || regen_menu.get()>
                            <RegenMenu id=id_sv.get_value() open=regen_menu/>
                        </Show>
                    </span>
                    {can_speak.then(|| view! {
                        <button class="nc-icon-btn sm" class:is-on=move || speaking.get() title="Read aloud" aria-label="Read aloud" on:click=move |_| {
                            if speaking.get_untracked() {
                                bridge::stop_speaking();
                                speaking.set(false);
                            } else {
                                let done = Closure::<dyn FnMut()>::new(move || speaking.set(false));
                                if bridge::speak(&text.get_untracked(), &done) {
                                    speaking.set(true);
                                }
                                done.forget();
                            }
                        }>
                            <Icon name="speaker" size=16/>
                        </button>
                    })}
                    <button class="nc-icon-btn sm" title="Delete" aria-label="Delete" on:click=move |_| state.delete_message(id_sv.get_value())>
                        <Icon name="trash" size=16/>
                    </button>
                </span>
                <span class="nc-msg-meta">
                    {move || {
                        let (_, duration, usage, ttft) = meta.get();
                        let mut items = Vec::new();
                        if let Some(d) = duration {
                            items.push(view! { <span title="Total time"><Icon name="clock" size=12/>{format_duration_ms(d)}</span> }.into_any());
                        }
                        if let Some(u) = usage
                            && let Some(c) = u.completion_tokens {
                                let tps = duration.zip(ttft).map(|(d, t)| d.saturating_sub(t)).filter(|ms| *ms > 200).map(|ms| c as f64 / (ms as f64 / 1000.0));
                                let label = match tps {
                                    Some(t) => format!("{c} tokens, {t:.0}/s"),
                                    None => format!("{c} tokens"),
                                };
                                items.push(view! { <span title="Output tokens"><Icon name="zap" size=12/>{label}</span> }.into_any());
                            }
                        items
                    }}
                </span>
            </Show>
        </div>
    }
}

#[component]
pub fn AssistantMessage(sig: ArcRwSignal<Message>, in_group: bool) -> impl IntoView {
    let model = sig.with_untracked(|m| m.model.clone());
    let synthesis = sig.with_untracked(|m| m.synthesis);
    let live = {
        let sig = sig.clone();
        Memo::new(move |_| sig.with(|m| m.is_active()))
    };
    let status = {
        let sig = sig.clone();
        Memo::new(move |_| sig.with(|m| m.status))
    };
    let style = hue_style(model.as_ref());
    let name = model.as_ref().map(short_model).unwrap_or_else(|| "Assistant".into());
    let provider = model.as_ref().map(|m| m.provider.clone()).unwrap_or_default();
    let body_sig = sig.clone();
    let foot_sig = sig.clone();
    view! {
        <div class="nc-msg nc-msg-assistant nc-enter" class:is-live=move || live.get() style=style>
            <div class="nc-msg-head">
                <span class="nc-hue-dot lg"></span>
                <span class="nc-model-name nc-truncate" title=format!("{provider}/{name}")>{name.clone()}</span>
                {synthesis.then(|| view! { <span class="nc-tag">"Combined answer"</span> })}
                <Show when=move || status.get() == MessageStatus::Cancelled>
                    <span class="nc-tag">"Stopped"</span>
                </Show>
            </div>
            <AssistantBody sig=body_sig/>
            {(!in_group).then(|| view! { <MessageFooter sig=foot_sig.clone()/> })}
        </div>
    }
}
