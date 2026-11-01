//! Several models answering the same prompt: side-by-side columns on wide
//! screens, swipeable tabs on phones, plus the optional combined answer.

use leptos::prelude::*;
use novumchat_core::types::*;

use crate::components::message::{AssistantBody, AssistantMessage, BranchSwitcher, MessageFooter};
use crate::icons::Icon;
use crate::state::use_state;
use crate::util::{distinct_hues, short_model};

#[component]
fn Column(sig: ArcRwSignal<Message>, active: bool, hue: &'static str) -> impl IntoView {
    let state = use_state();
    let id = sig.with_untracked(|m| m.id.clone());
    let model = sig.with_untracked(|m| m.model.clone());
    let style = format!("--hue: {hue}");
    let name = model.as_ref().map(short_model).unwrap_or_default();
    let status = {
        let sig = sig.clone();
        Memo::new(move |_| sig.with(|m| m.status))
    };
    let expanded = RwSignal::new(false);
    let body = sig.clone();
    let foot = sig.clone();
    view! {
        <div class="nc-ens-col" class:is-active=active class:is-expanded=move || expanded.get() style=style>
            <div class="nc-msg-head" style="margin-bottom:8px">
                <span class="nc-hue-dot lg"></span>
                <span class="nc-model-name nc-truncate" style="flex:1">{name}</span>
                {move || match status.get() {
                    MessageStatus::Pending | MessageStatus::Streaming => view! { <span class="nc-spinner" style="color:var(--hue)"></span> }.into_any(),
                    MessageStatus::Error => view! { <span class="nc-badge danger">"Failed"</span> }.into_any(),
                    MessageStatus::Cancelled => view! { <span class="nc-badge">"Stopped"</span> }.into_any(),
                    MessageStatus::Complete => {
                        if active {
                            view! { <span class="nc-badge accent"><Icon name="check" size=12/>"In thread"</span> }.into_any()
                        } else {
                            let id = id.clone();
                            view! {
                                <button class="nc-btn nc-btn-ghost nc-btn-sm" style="height:26px;padding:0 8px" title="Continue the conversation from this answer" on:click=move |_| state.set_leaf(id.clone())>
                                    "Continue with this"
                                </button>
                            }
                            .into_any()
                        }
                    }
                }}
            </div>
            <div class="nc-msg nc-msg-assistant" style="padding-left:0">
                <div class="nc-ens-body">
                    <AssistantBody sig=body/>
                </div>
            </div>
            <MessageFooter sig=foot branches=false/>
            <button class="nc-btn nc-btn-ghost nc-btn-sm nc-only-desktop" style="align-self:flex-start;height:26px;padding:0 8px;margin-top:-2px" on:click=move |_| expanded.update(|e| *e = !*e)>
                {move || if expanded.get() { "Collapse" } else { "Show full answer" }}
            </button>
        </div>
    }
}

#[component]
pub fn Ensemble(group: Id, members: Vec<Id>, active: Id) -> impl IntoView {
    let state = use_state();
    let sigs: Vec<ArcRwSignal<Message>> = members.iter().filter_map(|id| state.message(id)).collect();
    let synth: Vec<ArcRwSignal<Message>> = sigs.iter().filter(|s| s.with_untracked(|m| m.synthesis)).cloned().collect();
    let answers: Vec<ArcRwSignal<Message>> =
        sigs.iter().filter(|s| !s.with_untracked(|m| m.synthesis)).cloned().collect();
    let synth_latest = synth.last().cloned();
    let count = answers.len();
    let hues = distinct_hues(&answers.iter().map(|s| s.with_untracked(|m| m.model.clone())).collect::<Vec<_>>());
    let hues_tabs = hues.clone();
    let hues_mobile = hues.clone();

    let live = {
        let all = sigs.clone();
        Memo::new(move |_| all.iter().any(|s| s.with(|m| m.is_active())))
    };
    let complete = {
        let a = answers.clone();
        Memo::new(move |_| a.iter().filter(|s| s.with(|m| m.status == MessageStatus::Complete)).count())
    };
    let has_synth = synth_latest.is_some();
    // Mobile tab: the active answer, else the first.
    let initial_tab = answers.iter().position(|s| s.with_untracked(|m| m.id == active)).unwrap_or(0);
    let tab = RwSignal::new(initial_tab);
    let touch_x = StoredValue::new(None::<f64>);

    let first_id = sigs.first().map(|s| s.with_untracked(|m| m.id.clone())).unwrap_or_default();
    let group_for_btn = group.clone();
    let answers_tabs = answers.clone();
    let answers_mobile = answers.clone();
    let active_for_cols = active.clone();
    let active_mobile = active.clone();

    view! {
        <section class="nc-ensemble nc-enter" class:is-live=move || live.get()>
            <div class="nc-ens-head">
                <Icon name="layers" size=15/>
                <b>{format!("{count} models")}</b>
                <span class="nc-ens-spectrum"></span>
                <BranchSwitcher id=first_id/>
            </div>
            {synth_latest.map(|s| view! {
                <div class="nc-synth">
                    <AssistantMessage sig=s in_group=false/>
                </div>
            })}
            // Desktop: columns.
            <div class="nc-ens-grid">
                {answers.iter().enumerate().map(|(i, s)| {
                    let is_active = s.with_untracked(|m| m.id == active_for_cols);
                    view! { <Column sig=s.clone() active=is_active hue=hues[i]/> }
                }).collect_view()}
            </div>
            // Phone: tabs + swipe.
            <div class="nc-ens-tabs" role="tablist">
                {answers_tabs.iter().enumerate().map(|(i, s)| {
                    let model = s.with_untracked(|m| m.model.clone());
                    let style = format!("--hue: {}", hues_tabs[i]);
                    let name = model.as_ref().map(short_model).unwrap_or_default();
                    let sig = s.clone();
                    let st = Memo::new(move |_| sig.with(|m| m.status));
                    view! {
                        <button role="tab" style=style class:is-active=move || tab.get() == i aria-selected=move || (tab.get() == i).to_string() on:click=move |_| tab.set(i)>
                            <span class="nc-hue-dot"></span>
                            {name}
                            <Show when=move || st.get().is_active()>
                                <span class="nc-spinner" style="width:11px;height:11px;color:var(--hue)"></span>
                            </Show>
                        </button>
                    }
                }).collect_view()}
            </div>
            <div
                class="nc-ens-mobile nc-only-mobile"
                on:touchstart=move |ev: web_sys::TouchEvent| {
                    touch_x.set_value(ev.touches().get(0).map(|t| t.client_x() as f64));
                }
                on:touchend=move |ev: web_sys::TouchEvent| {
                    let Some(x0) = touch_x.get_value() else { return };
                    let Some(t) = ev.changed_touches().get(0) else { return };
                    let dx = t.client_x() as f64 - x0;
                    if dx < -50.0 && tab.get_untracked() + 1 < count {
                        tab.update(|t| *t += 1);
                    } else if dx > 50.0 && tab.get_untracked() > 0 {
                        tab.update(|t| *t -= 1);
                    }
                    touch_x.set_value(None);
                }
            >
                {move || {
                    let i = tab.get();
                    answers_mobile.get(i).cloned().map(|s| {
                        let is_active = s.with_untracked(|m| m.id == active_mobile);
                        view! { <Column sig=s active=is_active hue=hues_mobile[i]/> }
                    })
                }}
            </div>
            <Show when={move || !has_synth && !live.get() && complete.get() >= 2}>
                <button class="nc-btn nc-btn-secondary nc-btn-sm" style="margin-top:12px" on:click={
                    let g = group_for_btn.clone();
                    move |_| state.synthesize(g.clone())
                }>
                    <Icon name="sparkles" size=15/>
                    "Combine into one answer"
                </button>
            </Show>
        </section>
    }
}
