//! Message rendering: Markdown (+ LaTeX, code, artifacts) → sanitised HTML,
//! then DOM post-processing for live previews.

use std::rc::Rc;

use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use novumchat_core::markdown::{Artifact, ArtifactKind, RenderHooks, RenderOptions, Rendered, render_markdown};
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, HtmlIFrameElement};

use crate::bridge;
use crate::icons::icon_svg;
use crate::state::{AppState, Fullscreen, use_state};

struct JsHooks;

impl RenderHooks for JsHooks {
    fn math(&self, tex: &str, display: bool) -> Option<String> {
        bridge::render_math(tex, display)
    }
    fn highlight(&self, code: &str, lang: &str) -> Option<String> {
        bridge::highlight(code, lang)
    }
}

pub fn render(md: &str, opts: RenderOptions) -> Rendered {
    let mut r = render_markdown(md, &JsHooks, opts);
    r.html = bridge::sanitize(&r.html, opts.load_remote_images);
    r
}

fn document() -> web_sys::Document {
    web_sys::window().and_then(|w| w.document()).expect("document")
}

const SANDBOX: &str = "allow-scripts allow-forms allow-modals allow-pointer-lock allow-downloads";

fn make_iframe(doc_html: &str, title: &str) -> Option<HtmlIFrameElement> {
    let el = document().create_element("iframe").ok()?.dyn_into::<HtmlIFrameElement>().ok()?;
    let _ = el.set_attribute("sandbox", SANDBOX);
    let _ = el.set_attribute("referrerpolicy", "no-referrer");
    let _ = el.set_attribute("title", title);
    let _ = el.set_attribute("allow", "fullscreen; clipboard-write; accelerometer; gyroscope");
    el.set_srcdoc(doc_html);
    Some(el)
}

fn kind_str(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Html => "html",
        ArtifactKind::Svg => "svg",
        ArtifactKind::Mermaid => "mermaid",
        ArtifactKind::Javascript => "javascript",
    }
}

fn mount_frame(container: &Element, art: &Artifact, network: bool, dark: bool) {
    let doc = bridge::artifact_doc(&art.code, kind_str(art.kind), network, dark);
    if let Some(frame) = make_iframe(&doc, &art.title) {
        container.set_inner_html("");
        let _ = container.append_child(&frame);
    }
}

fn network_button(on: bool) -> String {
    let (icon, label, title) = if on {
        ("wifi", "Online", "This preview may load resources from the internet. Click to block network access.")
    } else {
        (
            "wifi-off",
            "Offline",
            "Network access is blocked for safety. Click to let this preview load resources from the internet.",
        )
    };
    format!(
        "<button type=\"button\" class=\"nc-net-toggle{}\" data-action=\"artifact-network\" title=\"{title}\">{}<span>{label}</span></button>",
        if on { " is-on" } else { "" },
        icon_svg(icon, 13)
    )
}

/// Wires previews into freshly inserted HTML.
fn post_process(root: &Element, artifacts: &[Artifact], network_default: bool, dark: bool) {
    let Ok(nodes) = root.query_selector_all(".nc-artifact") else { return };
    for i in 0..nodes.length() {
        let Some(node) = nodes.item(i).and_then(|n| n.dyn_into::<Element>().ok()) else { continue };
        let Some(idx) = node.get_attribute("data-artifact").and_then(|s| s.parse::<usize>().ok()) else { continue };
        let Some(art) = artifacts.get(idx) else { continue };
        match art.kind {
            ArtifactKind::Html | ArtifactKind::Svg => {
                let net = node.get_attribute("data-network").map(|v| v == "1").unwrap_or(network_default);
                let _ = node.set_attribute("data-network", if net { "1" } else { "0" });
                if let Ok(Some(actions)) = node.query_selector(".nc-artifact-actions")
                    && actions.query_selector("[data-action=artifact-network]").ok().flatten().is_none()
                {
                    let _ = actions.insert_adjacent_html("afterbegin", &network_button(net));
                }
                if let Ok(Some(frame)) = node.query_selector("[data-frame]") {
                    mount_frame(&frame, art, net, dark);
                }
            }
            ArtifactKind::Mermaid => {
                if let Ok(Some(target)) = node.query_selector("[data-mermaid]") {
                    let code = art.code.clone();
                    spawn_local(async move {
                        match bridge::await_promise(bridge::mermaid(&code, dark)).await {
                            Ok(svg) => {
                                target.set_inner_html(&bridge::sanitize_svg(&svg.as_string().unwrap_or_default()))
                            }
                            Err(e) => target.set_inner_html(&format!(
                                "<div class=\"nc-error\" style=\"margin:0\">Diagram error: {}</div>",
                                novumchat_core::markdown::escape_html(&e)
                            )),
                        }
                    });
                }
            }
            ArtifactKind::Javascript => {}
        }
    }
}

fn closest(target: &Element, selector: &str) -> Option<Element> {
    target.closest(selector).ok().flatten()
}

fn flash_done(btn: &Element) {
    let _ = btn.class_list().add_1("is-done");
    let original = btn.inner_html();
    btn.set_inner_html(&format!("{}<span>Copied</span>", icon_svg("check", 14)));
    let btn = btn.clone();
    set_timeout(
        move || {
            let _ = btn.class_list().remove_1("is-done");
            btn.set_inner_html(&original);
        },
        std::time::Duration::from_millis(1400),
    );
}

/// Handles clicks inside rendered content (event delegation).
fn handle_click(ev: &web_sys::MouseEvent, state: AppState, artifacts: &[Artifact], dark: bool) {
    let Some(target) = ev.target().and_then(|t| t.dyn_into::<Element>().ok()) else { return };
    if let Some(a) = closest(&target, "a[href]")
        && closest(&target, "[data-action]").is_none()
    {
        ev.prevent_default();
        if let Some(href) = a.get_attribute("href") {
            if href.starts_with('#') {
                return;
            }
            state.open_link(href);
        }
        return;
    }
    let Some(btn) = closest(&target, "[data-action]") else { return };
    let action = btn.get_attribute("data-action").unwrap_or_default();
    let artifact_el = closest(&btn, ".nc-artifact");
    let artifact = artifact_el
        .as_ref()
        .and_then(|el| el.get_attribute("data-artifact"))
        .or_else(|| btn.get_attribute("data-artifact"))
        .and_then(|s| s.parse::<usize>().ok())
        .and_then(|i| artifacts.get(i).cloned());
    match action.as_str() {
        "copy-code" => {
            ev.prevent_default();
            let text = match &artifact {
                Some(a) if artifact_el.is_some() => a.code.clone(),
                _ => closest(&btn, ".nc-code")
                    .and_then(|c| c.query_selector("code").ok().flatten())
                    .and_then(|c| c.text_content())
                    .unwrap_or_default(),
            };
            let btn = btn.clone();
            spawn_local(async move {
                if bridge::copy(&text).await {
                    flash_done(&btn);
                }
            });
        }
        "artifact-preview" | "artifact-code" => {
            if let Some(el) = &artifact_el {
                let view = if action == "artifact-code" { "code" } else { "preview" };
                let _ = el.set_attribute("data-view", view);
                if let Ok(tabs) = el.query_selector_all(".nc-artifact-tabs button") {
                    for i in 0..tabs.length() {
                        if let Some(t) = tabs.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
                            let on = t.get_attribute("data-action").as_deref() == Some(action.as_str());
                            let _ = if on {
                                t.class_list().add_1("is-active")
                            } else {
                                t.class_list().remove_1("is-active")
                            };
                        }
                    }
                }
            }
        }
        "artifact-reload" | "artifact-network" => {
            if let (Some(el), Some(art)) = (&artifact_el, &artifact) {
                let mut net = el.get_attribute("data-network").as_deref() == Some("1");
                if action == "artifact-network" {
                    net = !net;
                    let _ = el.set_attribute("data-network", if net { "1" } else { "0" });
                    btn.set_outer_html(&network_button(net));
                }
                if let Ok(Some(frame)) = el.query_selector("[data-frame]") {
                    mount_frame(&frame, art, net, dark);
                }
            }
        }
        "artifact-fullscreen" => {
            if let (Some(el), Some(art)) = (&artifact_el, &artifact) {
                let net = el.get_attribute("data-network").as_deref() == Some("1");
                state.fullscreen.set(Some(Fullscreen {
                    code: art.code.clone(),
                    kind: kind_str(art.kind).into(),
                    title: art.title.clone(),
                    network: net,
                }));
            }
        }
        "run-js" => {
            if let Some(art) = &artifact {
                let idx = btn.get_attribute("data-artifact").unwrap_or_default();
                let wrapper = closest(&btn, ".nc-js");
                if let Some(out) =
                    wrapper.and_then(|w| w.query_selector(&format!("[data-output=\"{idx}\"]")).ok().flatten())
                    && let Some(frame) = make_iframe(&bridge::js_runner_doc(&art.code, dark), "JavaScript output")
                {
                    let _ = web_sys::HtmlElement::style(&frame).set_property("height", "60px");
                    out.set_inner_html("");
                    let _ = out.append_child(&frame);
                }
            }
        }
        "load-image" => {
            if let Some(src) = btn.get_attribute("data-src")
                && let Ok(img) = document().create_element("img")
            {
                let _ = img.set_attribute("src", &src);
                let _ = img.set_attribute("alt", &btn.get_attribute("data-alt").unwrap_or_default());
                let _ = img.set_attribute("class", "nc-img");
                let _ = img.set_attribute("referrerpolicy", "no-referrer");
                let _ = btn.replace_with_with_node_1(&img);
            }
        }
        _ => {}
    }
}

/// Resizes JS-runner iframes from their `postMessage` height reports.
pub fn install_runner_resizer() {
    let cb =
        wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
            let data = ev.data();
            let is_runner = js_sys::Reflect::get(&data, &"novumRunner".into()).ok().is_some_and(|v| v.is_truthy());
            if !is_runner {
                return;
            }
            let h = js_sys::Reflect::get(&data, &"h".into()).ok().and_then(|v| v.as_f64()).unwrap_or(60.0);
            let Some(source) = ev.source() else { return };
            let Ok(frames) = document().query_selector_all(".nc-js-output iframe") else { return };
            for i in 0..frames.length() {
                if let Some(f) = frames.item(i).and_then(|n| n.dyn_into::<HtmlIFrameElement>().ok())
                    && let Some(win) = f.content_window()
                    && js_sys::Object::is(win.as_ref(), source.as_ref())
                {
                    let _ =
                        web_sys::HtmlElement::style(&f).set_property("height", &format!("{}px", h.clamp(40.0, 420.0)));
                }
            }
        });
    if let Some(w) = web_sys::window() {
        let _ = w.add_event_listener_with_callback("message", cb.as_ref().unchecked_ref());
    }
    cb.forget();
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Renders Markdown. While `live`, re-rendering is throttled and previews wait.
#[component]
pub fn Markdown(#[prop(into)] text: Signal<String>, #[prop(into)] live: Signal<bool>) -> impl IntoView {
    let state = use_state();
    let node: NodeRef<html::Div> = NodeRef::new();
    let artifacts: StoredValue<Rc<Vec<Artifact>>, LocalStorage> = StoredValue::new_local(Rc::new(Vec::new()));
    let last_render = StoredValue::new(0.0f64);
    let pending = StoredValue::new(false);
    let last_key = StoredValue::new(String::new());

    let opts = move || {
        let (previews, remote) = state.config.with_untracked(|c| {
            c.as_ref().map(|c| (c.appearance.live_previews, c.appearance.load_remote_images)).unwrap_or((true, false))
        });
        RenderOptions { streaming: live.get_untracked(), live_previews: previews, load_remote_images: remote }
    };

    let do_render = move || {
        let Some(el) = node.get_untracked() else { return };
        let md = text.get_untracked();
        let o = opts();
        let dark = state.dark.get_untracked();
        // Diagrams take their palette from the theme and accent, so both are
        // part of the key.
        let accent =
            state.config.with_untracked(|c| c.as_ref().map(|c| c.appearance.accent.clone()).unwrap_or_default());
        let key =
            format!("{}|{}|{}|{}|{accent}|{:x}", o.streaming, o.live_previews, o.load_remote_images, dark, fnv(&md));
        if key == last_key.get_value() && !o.streaming {
            return;
        }
        last_key.set_value(key);
        let r = render(&md, o);
        let el: &web_sys::HtmlDivElement = &el;
        el.set_inner_html(&r.html);
        if !o.streaming {
            post_process(el, &r.artifacts, false, dark);
        }
        artifacts.set_value(Rc::new(r.artifacts));
        last_render.set_value(js_sys::Date::now());
    };

    Effect::new(move |_| {
        // Track inputs.
        text.track();
        let is_live = live.get();
        state.dark.track();
        state.config.with(|c| c.as_ref().map(|c| (c.appearance.live_previews, c.appearance.load_remote_images)));
        if !is_live {
            pending.set_value(false);
            do_render();
            return;
        }
        let since = js_sys::Date::now() - last_render.get_value();
        const FRAME_MS: f64 = 70.0;
        if since >= FRAME_MS {
            do_render();
        } else if !pending.get_value() {
            pending.set_value(true);
            set_timeout(
                move || {
                    if pending.get_value() {
                        pending.set_value(false);
                        do_render();
                    }
                },
                std::time::Duration::from_millis((FRAME_MS - since).max(10.0) as u64),
            );
        }
    });

    let on_click = move |ev: web_sys::MouseEvent| {
        let arts = artifacts.get_value();
        handle_click(&ev, state, &arts, state.dark.get_untracked());
    };

    view! { <div class="nc-md" node_ref=node on:click=on_click></div> }
}

/// Fullscreen view of an artifact.
#[component]
pub fn FullscreenArtifact() -> impl IntoView {
    let state = use_state();
    let frame: NodeRef<html::Div> = NodeRef::new();
    Effect::new(move |_| {
        let Some(fs) = state.fullscreen.get() else { return };
        let Some(el) = frame.get() else { return };
        let doc = bridge::artifact_doc(&fs.code, &fs.kind, fs.network, state.dark.get_untracked());
        if let Some(f) = make_iframe(&doc, &fs.title) {
            el.set_inner_html("");
            let _ = el.append_child(&f);
            let _ = web_sys::HtmlElement::style(&f).set_property("flex", "1");
            let _ = web_sys::HtmlElement::style(&f).set_property("width", "100%");
            let _ = web_sys::HtmlElement::style(&f).set_property("height", "100%");
        }
    });
    view! {
        <Show when=move || state.fullscreen.with(|f| f.is_some())>
            <div class="nc-fullscreen" role="dialog" aria-modal="true">
                <div class="nc-fullscreen-bar">
                    <span class="nc-artifact-icon">
                        <crate::icons::Icon name="sparkles" size=18/>
                    </span>
                    <b class="nc-truncate" style="flex:1">
                        {move || state.fullscreen.with(|f| f.as_ref().map(|f| f.title.clone()).unwrap_or_default())}
                    </b>
                    <button class="nc-btn nc-btn-secondary nc-btn-sm" on:click=move |_| state.fullscreen.set(None)>
                        <crate::icons::Icon name="x" size=16/>
                        "Close"
                    </button>
                </div>
                <div node_ref=frame style="flex:1;display:flex;min-height:0"></div>
            </div>
        </Show>
    }
}

/// Focuses an element after the next frame.
pub fn focus_later(el: HtmlElement) {
    request_animation_frame(move || {
        let _ = el.focus();
    });
}
