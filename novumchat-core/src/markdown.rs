//! Markdown → HTML for chat messages.
//!
//! Handles GitHub-flavoured Markdown, LaTeX math (`$…$`, `$$…$$`, `\(…\)`,
//! `\[…\]`, ```` ```math ```` / ```` ```latex ```` blocks), syntax-highlighted
//! code blocks and *artifacts*: ```` ```html ````/```` ```svg ```` blocks become
//! live, sandboxed previews, ```` ```mermaid ```` becomes a diagram and
//! ```` ```js ```` blocks get a Run button.
//!
//! Rendering of math and highlighting is delegated to [`RenderHooks`] so the
//! UI can call KaTeX / highlight.js while tests use plain fallbacks. The output
//! is **not** sanitised; the UI must pass it through its HTML sanitiser.

use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};

/// Callbacks into the host for math and syntax highlighting.
pub trait RenderHooks {
    /// TeX → HTML. `None` falls back to the escaped source.
    fn math(&self, tex: &str, display: bool) -> Option<String>;
    /// Code → highlighted, *escaped* HTML. `None` falls back to plain escaping.
    fn highlight(&self, code: &str, lang: &str) -> Option<String>;
}

/// Hooks that render everything as escaped text.
pub struct PlainHooks;

impl RenderHooks for PlainHooks {
    fn math(&self, _tex: &str, _display: bool) -> Option<String> {
        None
    }
    fn highlight(&self, _code: &str, _lang: &str) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    /// The message is still streaming: previews are deferred until it is done.
    pub streaming: bool,
    /// Turn html/svg/mermaid blocks into live previews.
    pub live_previews: bool,
    /// Load `http(s)` images directly instead of showing a click-to-load chip.
    pub load_remote_images: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { streaming: false, live_previews: true, load_remote_images: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Html,
    Svg,
    Mermaid,
    Javascript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub kind: ArtifactKind,
    pub code: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rendered {
    pub html: String,
    /// Indexed by the `data-artifact` attribute in the HTML.
    pub artifacts: Vec<Artifact>,
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
}

/// Renders a chat message.
pub fn render_markdown(src: &str, hooks: &dyn RenderHooks, opts: RenderOptions) -> Rendered {
    let normalized = normalize_math_delimiters(src);
    let parser = Parser::new_ext(&normalized, options());

    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut events: Vec<Event<'_>> = Vec::new();

    // Code block state.
    let mut in_code: Option<String> = None; // language
    let mut code_buf = String::new();
    // Image state: (dest, title, alt text)
    let mut image: Option<(String, String, String)> = None;

    for event in parser {
        if let Some((_, _, alt)) = image.as_mut() {
            match event {
                Event::End(TagEnd::Image) => {
                    let (dest, title, alt) = image.take().unwrap_or_default();
                    events.push(Event::Html(CowStr::from(render_image(&dest, &title, &alt, opts))));
                }
                Event::Text(t) | Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => alt.push_str(&t),
                _ => {}
            }
            continue;
        }
        if in_code.is_some() {
            match event {
                Event::Text(t) => code_buf.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let lang = in_code.take().unwrap_or_default();
                    let code = std::mem::take(&mut code_buf);
                    let html = render_code_block(&lang, &code, hooks, opts, &mut artifacts);
                    events.push(Event::Html(CowStr::from(html)));
                }
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase(),
                    CodeBlockKind::Indented => String::new(),
                };
                in_code = Some(lang);
                code_buf.clear();
            }
            Event::Start(Tag::Image { dest_url, title, .. }) => {
                image = Some((dest_url.to_string(), title.to_string(), String::new()));
            }
            Event::InlineMath(tex) => {
                events.push(Event::Html(CowStr::from(render_math(&tex, false, hooks))));
            }
            Event::DisplayMath(tex) => {
                events.push(Event::Html(CowStr::from(render_math(&tex, true, hooks))));
            }
            Event::Start(Tag::Table(al)) => {
                events.push(Event::Html(CowStr::Borrowed("<div class=\"nc-table-wrap\">")));
                events.push(Event::Start(Tag::Table(al)));
            }
            Event::End(TagEnd::Table) => {
                events.push(Event::End(TagEnd::Table));
                events.push(Event::Html(CowStr::Borrowed("</div>")));
            }
            other => events.push(other),
        }
    }
    // Unterminated code block (still streaming).
    if let Some(lang) = in_code.take() {
        let html = render_code_block(&lang, &code_buf, hooks, opts, &mut artifacts);
        events.push(Event::Html(CowStr::from(html)));
    }

    let mut html = String::with_capacity(src.len() * 2);
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    Rendered { html, artifacts }
}

fn render_math(tex: &str, display: bool, hooks: &dyn RenderHooks) -> String {
    let tex = tex.trim();
    let inner = hooks.math(tex, display).unwrap_or_else(|| escape_html(tex));
    if display {
        // A span (styled as a block) stays valid inside the surrounding <p>.
        format!("<span class=\"nc-math nc-math-display\">{inner}</span>")
    } else {
        format!("<span class=\"nc-math\">{inner}</span>")
    }
}

fn render_image(dest: &str, title: &str, alt: &str, opts: RenderOptions) -> String {
    let lower = dest.trim().to_ascii_lowercase();
    let remote = lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("//");
    let safe_inline = lower.starts_with("data:image/") && !lower.starts_with("data:image/svg");
    let alt_e = escape_html(alt);
    let title_attr = if title.is_empty() { String::new() } else { format!(" title=\"{}\"", escape_html(title)) };
    if safe_inline || (remote && opts.load_remote_images) {
        format!("<img src=\"{}\" alt=\"{alt_e}\"{title_attr} loading=\"lazy\" class=\"nc-img\">", escape_html(dest))
    } else if remote {
        let host = dest.split("//").nth(1).and_then(|r| r.split('/').next()).unwrap_or(dest);
        format!(
            "<button type=\"button\" class=\"nc-img-blocked\" data-action=\"load-image\" data-src=\"{}\" data-alt=\"{alt_e}\" title=\"Load image from {}\">\
<span class=\"nc-img-blocked-icon\">◩</span><span>{}</span><small>{}</small></button>",
            escape_html(dest),
            escape_html(host),
            if alt.is_empty() { "Remote image".to_string() } else { alt_e.clone() },
            escape_html(host)
        )
    } else {
        format!("<span class=\"nc-img-missing\">[{}]</span>", if alt.is_empty() { "image".into() } else { alt_e })
    }
}

const COPY_ICON: &str = "<svg viewBox=\"0 0 24 24\" width=\"14\" height=\"14\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><rect x=\"9\" y=\"9\" width=\"12\" height=\"12\" rx=\"2\"/><path d=\"M5 15V5a2 2 0 0 1 2-2h10\"/></svg>";
const PLAY_ICON: &str = "<svg viewBox=\"0 0 24 24\" width=\"14\" height=\"14\" fill=\"currentColor\" aria-hidden=\"true\"><path d=\"M8 5.5v13a1 1 0 0 0 1.5.86l10.5-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5z\"/></svg>";
const EXPAND_ICON: &str = "<svg viewBox=\"0 0 24 24\" width=\"14\" height=\"14\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><path d=\"M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7\"/></svg>";
const RELOAD_ICON: &str = "<svg viewBox=\"0 0 24 24\" width=\"14\" height=\"14\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><path d=\"M21 12a9 9 0 1 1-2.64-6.36L21 8\"/><path d=\"M21 3v5h-5\"/></svg>";
const SPARK_ICON: &str = "<svg viewBox=\"0 0 24 24\" width=\"16\" height=\"16\" fill=\"currentColor\" aria-hidden=\"true\"><path d=\"M12 2l1.9 5.6L19.5 9.5l-5.6 1.9L12 17l-1.9-5.6L4.5 9.5l5.6-1.9z\"/><path d=\"M19 15l.9 2.6 2.6.9-2.6.9L19 22l-.9-2.6-2.6-.9 2.6-.9z\" opacity=\".7\"/></svg>";

fn lang_label(lang: &str) -> &str {
    match lang {
        "" => "text",
        "js" => "javascript",
        "ts" => "typescript",
        "py" => "python",
        "rs" => "rust",
        "sh" | "shell" | "zsh" | "console" => "bash",
        "yml" => "yaml",
        "md" => "markdown",
        "htm" => "html",
        other => other,
    }
}

fn plain_code_block(lang: &str, code: &str, hooks: &dyn RenderHooks, extra_actions: &str, extra_class: &str) -> String {
    let label = lang_label(lang);
    let body = if lang.is_empty() {
        escape_html(code)
    } else {
        hooks.highlight(code, label).unwrap_or_else(|| escape_html(code))
    };
    format!(
        "<div class=\"nc-code{extra_class}\" data-lang=\"{lang_e}\"><div class=\"nc-code-head\"><span class=\"nc-code-lang\">{label_e}</span>\
<span class=\"nc-code-actions\">{extra_actions}<button type=\"button\" class=\"nc-code-btn\" data-action=\"copy-code\" title=\"Copy code\">{COPY_ICON}<span>Copy</span></button></span></div>\
<pre><code class=\"hljs language-{lang_e}\">{body}</code></pre></div>",
        lang_e = escape_html(label),
        label_e = escape_html(label),
    )
}

fn looks_like_html_document(code: &str) -> bool {
    let head = code.trim_start().get(..64).unwrap_or(code.trim_start()).to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html")
}

fn html_title(code: &str) -> Option<String> {
    let lower = code.to_ascii_lowercase();
    let start = lower.find("<title>")? + "<title>".len();
    let end = lower[start..].find("</title>")? + start;
    let t = code.get(start..end)?.trim();
    (!t.is_empty()).then(|| crate::util::truncate_chars(t, 60))
}

fn render_code_block(
    lang: &str,
    code: &str,
    hooks: &dyn RenderHooks,
    opts: RenderOptions,
    artifacts: &mut Vec<Artifact>,
) -> String {
    let lang = if lang.is_empty() && looks_like_html_document(code) { "html" } else { lang };
    match lang {
        "math" | "latex" | "tex" | "katex"
            if !code.contains("\\documentclass") && !code.contains("\\begin{document}") =>
        {
            return render_math(code, true, hooks);
        }
        _ => {}
    }
    let previews = opts.live_previews && !opts.streaming;
    let kind = match lang {
        "html" | "htm" | "xhtml" => Some(ArtifactKind::Html),
        "svg" => Some(ArtifactKind::Svg),
        "xml" if code.trim_start().starts_with("<svg") => Some(ArtifactKind::Svg),
        "mermaid" => Some(ArtifactKind::Mermaid),
        "javascript" | "js" => Some(ArtifactKind::Javascript),
        _ => None,
    };
    let Some(kind) = kind else {
        return plain_code_block(lang, code, hooks, "", "");
    };
    if opts.streaming
        && matches!(kind, ArtifactKind::Html | ArtifactKind::Svg | ArtifactKind::Mermaid)
        && opts.live_previews
    {
        // Show the code while it streams, with a hint that a preview follows.
        let badge = "<span class=\"nc-code-badge\">preview when complete</span>";
        return plain_code_block(lang, code, hooks, badge, " nc-code-pending");
    }
    if !previews && kind != ArtifactKind::Javascript {
        return plain_code_block(lang, code, hooks, "", "");
    }
    let index = artifacts.len();
    match kind {
        ArtifactKind::Javascript => {
            if opts.streaming {
                return plain_code_block(lang, code, hooks, "", "");
            }
            artifacts.push(Artifact { kind, code: code.to_string(), title: "JavaScript".into() });
            let run = format!(
                "<button type=\"button\" class=\"nc-code-btn nc-code-run\" data-action=\"run-js\" data-artifact=\"{index}\" title=\"Run in a sandbox\">{PLAY_ICON}<span>Run</span></button>"
            );
            let block = plain_code_block(lang, code, hooks, &run, "");
            format!(
                "<div class=\"nc-js\" data-artifact=\"{index}\">{block}<div class=\"nc-js-output\" data-output=\"{index}\"></div></div>"
            )
        }
        ArtifactKind::Mermaid => {
            artifacts.push(Artifact { kind, code: code.to_string(), title: "Diagram".into() });
            let code_block = plain_code_block(lang, code, hooks, "", " nc-artifact-code");
            format!(
                "<div class=\"nc-artifact nc-artifact-mermaid\" data-artifact=\"{index}\" data-kind=\"mermaid\" data-view=\"preview\">\
<div class=\"nc-artifact-head\"><span class=\"nc-artifact-icon\">{SPARK_ICON}</span><span class=\"nc-artifact-title\">Diagram</span>\
<span class=\"nc-artifact-tabs\"><button type=\"button\" data-action=\"artifact-preview\" class=\"is-active\">Preview</button><button type=\"button\" data-action=\"artifact-code\">Code</button></span></div>\
<div class=\"nc-artifact-body\"><div class=\"nc-mermaid\" data-mermaid=\"{index}\"><div class=\"nc-artifact-loading\"></div></div>{code_block}</div></div>"
            )
        }
        ArtifactKind::Html | ArtifactKind::Svg => {
            let title = match kind {
                ArtifactKind::Svg => "SVG image".to_string(),
                _ => html_title(code).unwrap_or_else(|| "Interactive app".to_string()),
            };
            artifacts.push(Artifact { kind, code: code.to_string(), title: title.clone() });
            let code_block = plain_code_block(lang, code, hooks, "", " nc-artifact-code");
            let kind_s = if kind == ArtifactKind::Svg { "svg" } else { "html" };
            format!(
                "<div class=\"nc-artifact\" data-artifact=\"{index}\" data-kind=\"{kind_s}\" data-view=\"preview\">\
<div class=\"nc-artifact-head\"><span class=\"nc-artifact-icon\">{SPARK_ICON}</span><span class=\"nc-artifact-title\">{title_e}</span>\
<span class=\"nc-artifact-tabs\"><button type=\"button\" data-action=\"artifact-preview\" class=\"is-active\">Preview</button><button type=\"button\" data-action=\"artifact-code\">Code</button></span>\
<span class=\"nc-artifact-actions\"><button type=\"button\" class=\"nc-icon-btn\" data-action=\"artifact-reload\" title=\"Restart\">{RELOAD_ICON}</button>\
<button type=\"button\" class=\"nc-icon-btn\" data-action=\"artifact-fullscreen\" title=\"Full screen\">{EXPAND_ICON}</button>\
<button type=\"button\" class=\"nc-icon-btn\" data-action=\"copy-code\" title=\"Copy code\">{COPY_ICON}</button></span></div>\
<div class=\"nc-artifact-body\"><div class=\"nc-artifact-frame\" data-frame=\"{index}\"><div class=\"nc-artifact-loading\"></div></div>{code_block}</div></div>",
                title_e = escape_html(&title),
            )
        }
    }
}

/// Converts `\( … \)` → `$…$` and `\[ … \]` → `$$…$$` outside code, so that
/// the Markdown math extension picks them up, and wraps display environments
/// written without delimiters (`\begin{align} … \end{align}`). All common in
/// LLM output. Display math is joined onto one line so that a line inside it
/// that looks like Markdown (`- x`, `+ y`) cannot split it.
pub fn normalize_math_delimiters(src: &str) -> String {
    if !["\\(", "\\[", "math/tex", "$$", "\\begin{"].iter().any(|m| src.contains(m)) {
        return src.to_string();
    }
    convert_outside_code(src)
}

/// Display environments KaTeX renders, plus stand-ins for common ones it
/// doesn't support.
fn katex_display_env(name: &str) -> Option<&'static str> {
    Some(match name {
        "equation" => "equation",
        "equation*" | "displaymath" => "equation*",
        "align" | "eqnarray" | "flalign" => "align",
        "align*" | "eqnarray*" | "flalign*" => "align*",
        "alignat" => "alignat",
        "alignat*" => "alignat*",
        "gather" | "multline" => "gather",
        "gather*" | "multline*" => "gather*",
        _ => return None,
    })
}

/// A display environment written without `$$` at `i` (which starts with
/// `\begin{`): returns the wrapped math and the bytes consumed.
fn bare_environment_at(src: &str, i: usize) -> Option<(String, usize)> {
    let rest = &src[i..];
    let name_end = rest.find('}')?;
    let name = &rest["\\begin{".len()..name_end];
    let katex = katex_display_env(name)?;
    let end_tag = format!("\\end{{{name}}}");
    let close = rest.find(&end_tag)?;
    let body = &rest[name_end + 1..close];
    if body.trim().is_empty() || has_blank_line(body) {
        return None;
    }
    let math = format!("\\begin{{{katex}}}{}\\end{{{katex}}}", one_line(body));
    Some((format!("$${math}$$"), close + end_tag.len()))
}

fn has_blank_line(s: &str) -> bool {
    s.contains("\n\n") || s.contains("\n\r\n")
}

/// Joins display math onto one line. Line breaks are only whitespace in TeX;
/// `%` comments (which end at the line break) are dropped first.
fn one_line(tex: &str) -> String {
    tex.lines().map(|l| strip_tex_comment(l).trim()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

fn strip_tex_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'%' => return &line[..i],
            _ => i += 1,
        }
    }
    line
}

/// `<script type="math/tex">…</script>` (MathTeX / MathJax style) at `i`:
/// returns (tex, display, bytes consumed).
fn mathtex_script_at(src: &str, i: usize) -> Option<(String, bool, usize)> {
    let rest = &src[i..];
    let head = rest.get(..40)?.to_ascii_lowercase();
    if !head.starts_with("<script") || !head.contains("math/tex") {
        return None;
    }
    let tag_end = rest.find('>')?;
    let tag = rest[..tag_end].to_ascii_lowercase();
    if !tag.contains("type=\"math/tex") && !tag.contains("type='math/tex") && !tag.contains("type=math/tex") {
        return None;
    }
    let display = tag.contains("mode=display");
    let body_start = tag_end + 1;
    let close = rest[body_start..].to_ascii_lowercase().find("</script>")?;
    let tex = rest[body_start..body_start + close].trim().to_string();
    Some((tex, display, body_start + close + "</script>".len()))
}

fn convert_outside_code(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut fence: Option<(u8, usize)> = None;
    let mut line_start = true;
    while i < bytes.len() {
        // Fences at line starts.
        if line_start {
            let mut j = i;
            while j < bytes.len() && bytes[j] == b' ' && j - i < 4 {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'`' || bytes[j] == b'~') {
                let c = bytes[j];
                let n = bytes[j..].iter().take_while(|&&x| x == c).count();
                if n >= 3 {
                    let end = src[i..].find('\n').map(|p| i + p + 1).unwrap_or(bytes.len());
                    let rest_blank = src[j + n..end].trim().is_empty();
                    match fence {
                        None => fence = Some((c, n)),
                        Some((fc, fnn)) if fc == c && n >= fnn && rest_blank => fence = None,
                        _ => {}
                    }
                    out.push_str(&src[i..end]);
                    i = end;
                    line_start = true;
                    continue;
                }
            }
        }
        if fence.is_some() {
            let end = src[i..].find('\n').map(|p| i + p + 1).unwrap_or(bytes.len());
            out.push_str(&src[i..end]);
            i = end;
            line_start = true;
            continue;
        }
        let b = bytes[i];
        line_start = false;
        if b == b'\n' {
            out.push('\n');
            i += 1;
            line_start = true;
            continue;
        }
        if b == b'`' {
            // Inline code span: copy verbatim up to the matching run.
            let n = bytes[i..].iter().take_while(|&&x| x == b'`').count();
            let ticks = &src[i..i + n];
            if let Some(rel) = src[i + n..].find(ticks) {
                let end = i + n + rel + n;
                out.push_str(&src[i..end]);
                i = end;
            } else {
                out.push_str(ticks);
                i += n;
            }
            continue;
        }
        if b == b'<'
            && let Some((tex, display, used)) = mathtex_script_at(src, i)
        {
            if !tex.is_empty() {
                if display {
                    out.push_str("$$");
                    out.push_str(&tex);
                    out.push_str("$$");
                } else {
                    out.push('$');
                    out.push_str(&tex);
                    out.push('$');
                }
            }
            i += used;
            continue;
        }
        if b == b'$' && bytes.get(i + 1) == Some(&b'$') {
            if let Some(rel) = src[i + 2..].find("$$") {
                let inner = &src[i + 2..i + 2 + rel];
                if !has_blank_line(inner) {
                    out.push_str("$$");
                    out.push_str(&one_line(inner));
                    out.push_str("$$");
                    i = i + 2 + rel + 2;
                    continue;
                }
            }
            out.push_str("$$");
            i += 2;
            continue;
        }
        if b == b'\\'
            && src[i..].starts_with("\\begin{")
            && out.rsplit('\n').next().is_some_and(|l| l.trim().is_empty())
            && let Some((math, used)) = bare_environment_at(src, i)
        {
            out.push_str(&math);
            i += used;
            continue;
        }
        if b == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'\\' {
                out.push_str("\\\\");
                i += 2;
                continue;
            }
            let (close, display) = match next {
                b'(' => ("\\)", false),
                b'[' => ("\\]", true),
                _ => {
                    out.push('\\');
                    i += 1;
                    continue;
                }
            };
            if let Some(rel) = src[i + 2..].find(close) {
                let inner = &src[i + 2..i + 2 + rel];
                let blank_line = inner.contains("\n\n") || inner.contains("\n\r\n");
                if !blank_line && !inner.trim().is_empty() {
                    if display {
                        out.push_str("$$");
                        out.push_str(&one_line(inner));
                        out.push_str("$$");
                    } else {
                        out.push('$');
                        out.push_str(inner.trim());
                        out.push('$');
                    }
                    i = i + 2 + rel + 2;
                    continue;
                }
            }
            out.push('\\');
            i += 1;
            continue;
        }
        // Copy one UTF-8 character.
        let ch_len = utf8_len(b);
        out.push_str(&src[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHooks;
    impl RenderHooks for TestHooks {
        fn math(&self, tex: &str, display: bool) -> Option<String> {
            Some(format!("<katex display={display}>{}</katex>", escape_html(tex)))
        }
        fn highlight(&self, code: &str, lang: &str) -> Option<String> {
            Some(format!("<hl {lang}>{}</hl>", escape_html(code)))
        }
    }

    fn render(src: &str) -> Rendered {
        render_markdown(src, &TestHooks, RenderOptions::default())
    }

    #[test]
    fn basic_markdown() {
        let r = render("# Title\n\nSome **bold** and `code`.\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(r.html.contains("<h1>Title</h1>"));
        assert!(r.html.contains("<strong>bold</strong>"));
        assert!(r.html.contains("<code>code</code>"));
        assert!(r.html.contains("<div class=\"nc-table-wrap\"><table>"));
    }

    #[test]
    fn dollar_math() {
        let r = render("Euler: $e^{i\\pi}+1=0$ and\n\n$$\\int_0^1 x\\,dx$$\n");
        assert!(r.html.contains("<katex display=false>e^{i\\pi}+1=0</katex>"), "{}", r.html);
        assert!(r.html.contains("<katex display=true>\\int_0^1 x\\,dx</katex>"), "{}", r.html);
    }

    #[test]
    fn bracket_math_is_normalized() {
        let r = render("Inline \\( a^2 + b^2 \\) and block:\n\n\\[\nc^2\n\\]\n");
        assert!(r.html.contains("<katex display=false>a^2 + b^2</katex>"), "{}", r.html);
        assert!(r.html.contains("<katex display=true>c^2</katex>"), "{}", r.html);
    }

    #[test]
    fn math_inside_code_is_untouched() {
        let r = render("`\\(x\\)` and\n\n```python\nprint('\\(y\\)')\n```\n");
        assert!(r.html.contains("<code>\\(x\\)</code>"), "{}", r.html);
        assert!(r.html.contains("print("), "{}", r.html);
        assert!(!r.html.contains("katex"), "{}", r.html);
    }

    #[test]
    fn currency_is_not_math() {
        let r = render("It costs $5 and $10 today.");
        assert!(!r.html.contains("katex"), "{}", r.html);
    }

    #[test]
    fn code_blocks_are_highlighted_and_escaped() {
        let r = render("```rust\nfn main() { println!(\"<hi>\"); }\n```");
        assert!(r.html.contains("<hl rust>"), "{}", r.html);
        assert!(r.html.contains("&lt;hi&gt;"), "{}", r.html);
        assert!(r.html.contains("data-action=\"copy-code\""));
    }

    #[test]
    fn html_artifacts() {
        let src = "Here:\n\n```html\n<!doctype html><title>Clock</title><script>alert(1)</script>\n```\n";
        let r = render(src);
        assert_eq!(r.artifacts.len(), 1);
        assert_eq!(r.artifacts[0].kind, ArtifactKind::Html);
        assert_eq!(r.artifacts[0].title, "Clock");
        assert!(r.html.contains("data-frame=\"0\""));
        // The raw script never appears unescaped in the host document.
        assert!(!r.html.contains("<script>"));
        // While streaming, no artifact is produced.
        let s = render_markdown(src, &TestHooks, RenderOptions { streaming: true, ..Default::default() });
        assert!(s.artifacts.is_empty());
        assert!(s.html.contains("preview when complete"));
    }

    #[test]
    fn js_and_mermaid_and_latex_blocks() {
        let r =
            render("```js\nconsole.log(1)\n```\n\n```mermaid\ngraph TD; A-->B\n```\n\n```latex\n\\frac{a}{b}\n```\n");
        assert_eq!(r.artifacts.len(), 2);
        assert_eq!(r.artifacts[0].kind, ArtifactKind::Javascript);
        assert_eq!(r.artifacts[1].kind, ArtifactKind::Mermaid);
        assert!(r.html.contains("data-action=\"run-js\""));
        assert!(r.html.contains("<katex display=true>\\frac{a}{b}</katex>"), "{}", r.html);
    }

    #[test]
    fn unterminated_code_block_while_streaming() {
        let r =
            render_markdown("```python\nprint(1)", &TestHooks, RenderOptions { streaming: true, ..Default::default() });
        assert!(r.html.contains("print(1)"), "{}", r.html);
    }

    #[test]
    fn remote_images_are_blocked_by_default() {
        let r = render("![cat](https://example.com/cat.png)");
        assert!(r.html.contains("data-action=\"load-image\""), "{}", r.html);
        assert!(!r.html.contains("<img"), "{}", r.html);
        let r = render_markdown(
            "![cat](https://example.com/cat.png)",
            &TestHooks,
            RenderOptions { load_remote_images: true, ..Default::default() },
        );
        assert!(r.html.contains("<img src=\"https://example.com/cat.png\""), "{}", r.html);
    }

    #[test]
    fn utf8_is_preserved() {
        let r = render("Grüße \\(π\\) — 日本語 🎉");
        assert!(r.html.contains("Grüße"));
        assert!(r.html.contains("<katex display=false>π</katex>"), "{}", r.html);
        assert!(r.html.contains("日本語 🎉"));
    }

    #[test]
    fn bare_latex_environments_become_display_math() {
        let src = "The system:\n\n\\begin{align}\na &= b + c \\\\\n- d &= e\n\\end{align}\n\nDone.";
        let r = render(src);
        assert!(
            r.html.contains("<katex display=true>\\begin{align}a &amp;= b + c \\\\ - d &amp;= e\\end{align}</katex>"),
            "{}",
            r.html
        );
        assert!(!r.html.contains("<li>"), "{}", r.html);
        // Unsupported environments map to ones KaTeX knows.
        let r = render("\\begin{multline}\nx = 1\n\\end{multline}");
        assert!(r.html.contains("\\begin{gather}x = 1\\end{gather}"), "{}", r.html);
        // Already-delimited and mid-sentence environments are left alone.
        let r = render("$$\n\\begin{aligned}\nx &= 1\n\\end{aligned}\n$$");
        assert!(
            r.html.contains("<katex display=true>\\begin{aligned} x &amp;= 1 \\end{aligned}</katex>"),
            "{}",
            r.html
        );
        let r = render("Use `\\begin{align}` for aligned equations.");
        assert!(r.html.contains("<code>\\begin{align}</code>"), "{}", r.html);
    }

    #[test]
    fn display_math_is_not_split_by_markdown() {
        let r = render("$$\nx = 1\n- y\n$$");
        assert!(r.html.contains("<katex display=true>x = 1 - y</katex>"), "{}", r.html);
        // TeX comments end at the line break, so they are dropped first.
        let r = render("\\[\nx % note\n+ 1 \\% not a comment\n\\]");
        assert!(r.html.contains("<katex display=true>x + 1 \\% not a comment</katex>"), "{}", r.html);
    }

    #[test]
    fn mathtex_script_tags() {
        let r = render(
            "Inline <script type=\"math/tex\">x^2</script> and\n\n<script type=\"math/tex; mode=display\">\\sum_i i</script>\n",
        );
        assert!(r.html.contains("<katex display=false>x^2</katex>"), "{}", r.html);
        assert!(r.html.contains("<katex display=true>\\sum_i i</katex>"), "{}", r.html);
        assert!(!r.html.contains("<script"), "{}", r.html);
        // Left alone inside code.
        let r = render("```html\n<script type=\"math/tex\">y</script>\n```\n");
        assert!(r.html.contains("math/tex"), "{}", r.html);
        assert!(!r.html.contains("katex"), "{}", r.html);
    }

    #[test]
    fn gfm_alerts() {
        let r = render("> [!NOTE]\n> Hello");
        assert!(r.html.contains("markdown-alert-note"), "{}", r.html);
    }
}
