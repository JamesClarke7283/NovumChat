//! Inline SVG icons (24×24, stroke based) and the NovumChat prism mark.

use leptos::prelude::*;

fn path(name: &str) -> &'static str {
    match name {
        "plus" => r#"<path d="M12 5v14M5 12h14"/>"#,
        "search" => r#"<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>"#,
        "settings" => {
            r#"<path d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>"#
        }
        "sun" => {
            r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>"#
        }
        "moon" => r#"<path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"/>"#,
        "auto" => {
            r#"<circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 0 0 18z" fill="currentColor" stroke="none"/>"#
        }
        "lock" => r#"<rect x="4" y="11" width="16" height="10" rx="2.5"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>"#,
        "unlock" => r#"<rect x="4" y="11" width="16" height="10" rx="2.5"/><path d="M8 11V7a4 4 0 0 1 7.8-1.3"/>"#,
        "menu" => r#"<path d="M4 7h16M4 12h16M4 17h10"/>"#,
        "sidebar" => r#"<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M9 4v16"/>"#,
        "x" => r#"<path d="M18 6 6 18M6 6l12 12"/>"#,
        "send" => r#"<path d="M12 19V5M5 12l7-7 7 7"/>"#,
        "stop" => r#"<rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor"/>"#,
        "paperclip" => {
            r#"<path d="m21.4 11.6-9.2 9.2a6 6 0 0 1-8.5-8.5l9.2-9.2a4 4 0 0 1 5.7 5.7l-9.2 9.2a2 2 0 0 1-2.8-2.8l8.5-8.5"/>"#
        }
        "globe" => r#"<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>"#,
        "tool" => {
            r#"<path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2.4-.6-.6-2.4z"/>"#
        }
        "chevron-down" => r#"<path d="m6 9 6 6 6-6"/>"#,
        "chevron-right" => r#"<path d="m9 6 6 6-6 6"/>"#,
        "chevron-left" => r#"<path d="m15 6-6 6 6 6"/>"#,
        "copy" => r#"<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/>"#,
        "check" => r#"<path d="M20 6 9 17l-5-5"/>"#,
        "refresh" => r#"<path d="M21 12a9 9 0 1 1-2.6-6.4L21 8"/><path d="M21 3v5h-5"/>"#,
        "edit" => r#"<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/>"#,
        "trash" => {
            r#"<path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/>"#
        }
        "pin" => r#"<path d="M12 17v5M9 10.8V4h6v6.8l3 3.2H6z"/>"#,
        "more" => {
            r#"<circle cx="5" cy="12" r="1.4" fill="currentColor"/><circle cx="12" cy="12" r="1.4" fill="currentColor"/><circle cx="19" cy="12" r="1.4" fill="currentColor"/>"#
        }
        "speaker" => r#"<path d="M11 5 6 9H3v6h3l5 4z"/><path d="M15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13"/>"#,
        "sparkles" => {
            r#"<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"/><path d="M19 15l.8 2.2 2.2.8-2.2.8L19 21l-.8-2.2-2.2-.8 2.2-.8z"/>"#
        }
        "brain" => {
            r#"<path d="M9.5 3a3 3 0 0 0-3 3 3 3 0 0 0-2.5 4.5A3.5 3.5 0 0 0 5.5 17 3 3 0 0 0 9.5 21V3zM14.5 3a3 3 0 0 1 3 3 3 3 0 0 1 2.5 4.5 3.5 3.5 0 0 1-1.5 6.5 3 3 0 0 1-4 4V3z"/>"#
        }
        "eye" => r#"<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>"#,
        "file" => r#"<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>"#,
        "file-text" => {
            r#"<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M9 13h6M9 17h6"/>"#
        }
        "file-code" => {
            r#"<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M10 13l-2 2 2 2M14 13l2 2-2 2"/>"#
        }
        "file-edit" => {
            r#"<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h4"/><path d="M14 3v5h5"/><path d="M18.4 12.6a1.4 1.4 0 0 1 2 2L15 20l-3 .8.8-3z"/>"#
        }
        "image" => {
            r#"<rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="9" cy="9" r="2"/><path d="m21 15-4.5-4.5L6 21"/>"#
        }
        "folder" => r#"<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>"#,
        "terminal" => r#"<rect x="3" y="4" width="18" height="16" rx="3"/><path d="m7 9 3 3-3 3M13 15h4"/>"#,
        "download" => r#"<path d="M12 3v12M7 10l5 5 5-5M5 21h14"/>"#,
        "alert" => {
            r#"<path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z"/><path d="M12 9v4M12 17h.01"/>"#
        }
        "info" => r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>"#,
        "check-circle" => r#"<circle cx="12" cy="12" r="9"/><path d="m8 12 3 3 5-6"/>"#,
        "x-circle" => r#"<circle cx="12" cy="12" r="9"/><path d="m15 9-6 6M9 9l6 6"/>"#,
        "ban" => r#"<circle cx="12" cy="12" r="9"/><path d="m5.6 5.6 12.8 12.8"/>"#,
        "shield" => r#"<path d="M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6z"/>"#,
        "server" => {
            r#"<rect x="3" y="4" width="18" height="7" rx="2"/><rect x="3" y="13" width="18" height="7" rx="2"/><path d="M7 7.5h.01M7 16.5h.01"/>"#
        }
        "key" => r#"<circle cx="8" cy="15" r="4"/><path d="m10.8 12.2 8.2-8.2M17 6l2 2M15 8l2 2"/>"#,
        "layers" => r#"<path d="m12 3 9 5-9 5-9-5z"/><path d="m3 13 9 5 9-5"/>"#,
        "cpu" => {
            r#"<rect x="6" y="6" width="12" height="12" rx="2"/><path d="M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4"/>"#
        }
        "arrow-down" => r#"<path d="M12 5v14M5 12l7 7 7-7"/>"#,
        "external" => r#"<path d="M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#,
        "play" => {
            r#"<path d="M8 5.5v13a1 1 0 0 0 1.5.9l10.5-6.5a1 1 0 0 0 0-1.8L9.5 4.6A1 1 0 0 0 8 5.5z" fill="currentColor"/>"#
        }
        "zap" => r#"<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>"#,
        "message" => r#"<path d="M21 12a8 8 0 0 1-11.6 7.1L4 21l1.9-5.4A8 8 0 1 1 21 12z"/>"#,
        "palette" => {
            r#"<path d="M12 3a9 9 0 0 0 0 18c1 0 1.7-.8 1.7-1.7 0-.5-.2-.9-.5-1.2-.3-.3-.5-.7-.5-1.2 0-1 .8-1.7 1.7-1.7H16a5 5 0 0 0 5-5c0-4-4-7.2-9-7.2z"/><circle cx="7.5" cy="11" r="1.2" fill="currentColor"/><circle cx="10.5" cy="7.5" r="1.2" fill="currentColor"/><circle cx="15" cy="7.8" r="1.2" fill="currentColor"/>"#
        }
        "sliders" => {
            r#"<path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0"/><circle cx="16" cy="6" r="2"/><circle cx="10" cy="12" r="2"/><circle cx="18" cy="18" r="2"/>"#
        }
        "code" => r#"<path d="m16 18 6-6-6-6M8 6l-6 6 6 6"/>"#,
        "wifi" => {
            r#"<path d="M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0M2 9a15 15 0 0 1 20 0"/><path d="M12 19.5h.01"/>"#
        }
        "wifi-off" => {
            r#"<path d="m2 2 20 20M8.5 16a5 5 0 0 1 7 0M5 12.5a10 10 0 0 1 4.2-2.4M19 12.5a10 10 0 0 0-2.4-1.7M2 9a15 15 0 0 1 4.3-2.9M22 9a15 15 0 0 0-10-3.9"/><path d="M12 19.5h.01"/>"#
        }
        "branch" => {
            r#"<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="8" r="2"/><path d="M6 7v10M18 10a6 6 0 0 1-6 6H6"/>"#
        }
        "archive" => {
            r#"<rect x="3" y="4" width="18" height="5" rx="1.5"/><path d="M5 9v9a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V9M10 13h4"/>"#
        }
        "clock" => r#"<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>"#,
        "hash" => r#"<path d="M5 9h14M5 15h14M10 3 8 21M16 3l-2 18"/>"#,
        _ => r#"<circle cx="12" cy="12" r="9"/>"#,
    }
}

/// An inline icon.
#[component]
pub fn Icon(
    #[prop(into)] name: String,
    #[prop(default = 18)] size: u32,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let svg = format!(
        r#"<svg viewBox="0 0 24 24" width="{size}" height="{size}" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{}</svg>"#,
        path(&name)
    );
    view! { <span class=format!("nc-i {class}") style="display:inline-grid;place-items:center;line-height:0" inner_html=svg></span> }
}

/// SVG markup for places that need a string (e.g. dynamically built HTML).
pub fn icon_svg(name: &str, size: u32) -> String {
    format!(
        r#"<svg viewBox="0 0 24 24" width="{size}" height="{size}" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{}</svg>"#,
        path(name)
    )
}

/// The NovumChat mark: a beam refracted by a prism into the model spectrum.
#[component]
pub fn Mark(#[prop(default = 34)] size: u32, #[prop(default = false)] animate: bool) -> impl IntoView {
    let class = if animate { "nc-mark" } else { "nc-mark static" };
    let svg = format!(
        r#"<svg class="{class}" viewBox="0 0 64 64" width="{size}" height="{size}" aria-hidden="true" fill="none" stroke-linecap="round" stroke-linejoin="round">
<path class="beam" d="M5 31 L25 31" stroke="var(--text)" stroke-width="3"/>
<path class="prism" d="M30 13 L45 46 L15 46 Z" stroke="var(--text)" stroke-width="3.2"/>
<path class="ray" d="M37 31 L59 21" stroke="var(--s1)" stroke-width="2.8"/>
<path class="ray" d="M38 33 L59 30" stroke="var(--s2)" stroke-width="2.8"/>
<path class="ray" d="M38 35 L59 39" stroke="var(--s3)" stroke-width="2.8"/>
<path class="ray" d="M37 37 L57 48" stroke="var(--s4)" stroke-width="2.8"/>
</svg>"#
    );
    view! { <span style="display:inline-grid;line-height:0" inner_html=svg></span> }
}
