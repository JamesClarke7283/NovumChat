//! Small UI helpers.

use std::cell::RefCell;
use std::collections::HashMap;

use novumchat_core::types::ModelRef;

pub fn now_ms() -> i64 {
    js_sys::Date::now() as i64
}

const HUES: [&str; 5] = ["var(--s1)", "var(--s4)", "var(--s2)", "var(--s5)", "var(--s3)"];

thread_local! {
    /// Model → spectrum slot, assigned in the order models are first seen, so
    /// neighbouring models in lists get different colours.
    static HUE_SLOTS: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

fn hue_slot(model: &ModelRef) -> usize {
    HUE_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let next = slots.len() % HUES.len();
        *slots.entry(model.key()).or_insert(next)
    })
}

/// Registers models in display order (call when the model list loads).
pub fn register_models<'a>(models: impl IntoIterator<Item = &'a ModelRef>) {
    for m in models {
        hue_slot(m);
    }
}

/// Stable identity colour for a model (from the spectrum).
pub fn hue_for(model: &ModelRef) -> &'static str {
    HUES[hue_slot(model)]
}

/// Colours for models shown together (an ensemble): each one distinct.
pub fn distinct_hues(models: &[Option<ModelRef>]) -> Vec<&'static str> {
    let mut used = [false; 5];
    models
        .iter()
        .map(|m| {
            let mut slot = m.as_ref().map(hue_slot).unwrap_or(0);
            if used[slot]
                && let Some(free) = (0..HUES.len()).find(|i| !used[*i])
            {
                slot = free;
            }
            used[slot] = true;
            HUES[slot]
        })
        .collect()
}

pub fn hue_for_opt(model: Option<&ModelRef>) -> &'static str {
    model.map(hue_for).unwrap_or("var(--line-strong)")
}

pub fn hue_style(model: Option<&ModelRef>) -> String {
    format!("--hue: {}", hue_for_opt(model))
}

/// Sidebar section for a timestamp.
pub fn date_group(ts: i64) -> &'static str {
    let now = js_sys::Date::new_0();
    let start_of_today =
        js_sys::Date::new_with_year_month_day(now.get_full_year(), now.get_month() as i32, now.get_date() as i32)
            .get_time() as i64;
    let day = 86_400_000i64;
    if ts >= start_of_today {
        "Today"
    } else if ts >= start_of_today - day {
        "Yesterday"
    } else if ts >= start_of_today - 7 * day {
        "Previous 7 days"
    } else if ts >= start_of_today - 30 * day {
        "Previous 30 days"
    } else {
        "Older"
    }
}

pub fn greeting() -> &'static str {
    let h = js_sys::Date::new_0().get_hours();
    match h {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

pub fn is_narrow() -> bool {
    web_sys::window().and_then(|w| w.inner_width().ok()).and_then(|v| v.as_f64()).is_some_and(|w| w < 861.0)
}

pub fn coarse_pointer() -> bool {
    web_sys::window().and_then(|w| w.match_media("(pointer: coarse)").ok().flatten()).is_some_and(|m| m.matches())
}

pub fn short_model(m: &ModelRef) -> String {
    m.short_label()
}

pub fn file_icon(name: &str, mime: &str) -> &'static str {
    let lower = name.to_lowercase();
    if mime.starts_with("image/") {
        "image"
    } else if mime == "application/pdf" || lower.ends_with(".pdf") {
        "file-text"
    } else if novumchat_core::util::is_text_like(name, mime) {
        "file-code"
    } else {
        "file"
    }
}

/// `example.com` from a URL.
pub fn host_of(url: &str) -> String {
    let rest = url.split("//").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.trim_start_matches("www.").to_string()
}

/// Approximate token count used for the composer hint.
pub fn approx_tokens(chars: usize) -> usize {
    chars.div_ceil(4)
}
