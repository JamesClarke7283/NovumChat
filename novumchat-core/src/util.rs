//! Small pure helpers shared by backend and UI.

pub mod tree;

/// Lowercase ASCII slug made of `[a-z0-9-]`.
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if (c == '-' || c == '_' || c.is_whitespace() || c == '.') && !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Model name without registry/organisation prefixes:
/// `openai/gpt-4o` → `gpt-4o`, `hf.co/org/model:Q4` → `model:Q4`.
pub fn short_model_name(model: &str) -> String {
    model.rsplit('/').next().unwrap_or(model).to_string()
}

/// Truncates to at most `max` characters, appending `…` when shortened.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Initial chat title derived from the first user message.
pub fn title_from_text(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").trim_start_matches('#').trim();
    if line.is_empty() {
        return "New chat".into();
    }
    truncate_chars(line, 60)
}

/// Cleans a model-generated title: strips quotes, markdown and trailing punctuation.
pub fn clean_title(raw: &str) -> String {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches(|c: char| c == '#' || c == '*' || c.is_whitespace())
        .trim_start_matches("Title:")
        .trim_start_matches("title:")
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '*' || c == '`' || c == '“' || c == '”')
        .trim_end_matches(['.', '!', ':'])
        .trim();
    let line = line.replace("**", "").replace("__", "").replace('`', "");
    truncate_chars(line.trim(), 60)
}

/// `950ms`, `4.2s`, `2m 05s`.
pub fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let s = ms / 1000;
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

/// `1.4 KB`, `3.2 MB`.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

/// Makes a string safe as an OpenAI function name (`^[a-zA-Z0-9_-]{1,64}$`).
pub fn sanitize_tool_name(s: &str) -> String {
    let mut out: String =
        s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    if out.is_empty() {
        out.push('_');
    }
    out.truncate(64);
    out
}

/// Guesses whether a file is text from its MIME type or extension.
pub fn is_text_like(name: &str, mime: &str) -> bool {
    if mime.starts_with("text/") {
        return true;
    }
    const MIMES: &[&str] = &[
        "application/json",
        "application/xml",
        "application/javascript",
        "application/x-javascript",
        "application/typescript",
        "application/x-sh",
        "application/x-yaml",
        "application/yaml",
        "application/toml",
        "application/sql",
        "application/x-python",
        "application/ld+json",
        "image/svg+xml",
    ];
    if MIMES.contains(&mime) {
        return true;
    }
    const EXTS: &[&str] = &[
        "txt",
        "md",
        "markdown",
        "rst",
        "csv",
        "tsv",
        "json",
        "jsonl",
        "yaml",
        "yml",
        "toml",
        "ini",
        "cfg",
        "conf",
        "xml",
        "html",
        "htm",
        "css",
        "scss",
        "less",
        "js",
        "mjs",
        "cjs",
        "ts",
        "tsx",
        "jsx",
        "rs",
        "py",
        "rb",
        "go",
        "java",
        "kt",
        "kts",
        "swift",
        "c",
        "h",
        "cpp",
        "hpp",
        "cc",
        "cs",
        "php",
        "pl",
        "lua",
        "r",
        "jl",
        "scala",
        "sh",
        "bash",
        "zsh",
        "fish",
        "ps1",
        "bat",
        "sql",
        "graphql",
        "proto",
        "tex",
        "bib",
        "log",
        "env",
        "gitignore",
        "dockerfile",
        "makefile",
        "cmake",
        "gradle",
        "vue",
        "svelte",
        "astro",
        "dart",
        "ex",
        "exs",
        "erl",
        "hs",
        "ml",
        "nim",
        "zig",
        "v",
        "sol",
        "tf",
        "hcl",
        "nix",
        "lock",
        "svg",
        "org",
        "adoc",
    ];
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    EXTS.contains(&ext) || matches!(lower.as_str(), "dockerfile" | "makefile" | "readme" | "license")
}

/// Language tag for a fenced code block from a file name.
pub fn language_for_file(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    match lower.rsplit('.').next().unwrap_or("") {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "html" | "htm" => "html",
        "css" => "css",
        "json" | "jsonl" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "sh" | "bash" | "zsh" => "bash",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "sql" => "sql",
        "xml" | "svg" => "xml",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "lua" => "lua",
        "tex" => "latex",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("My Ollama Box"), "my-ollama-box");
        assert_eq!(slugify("  --LM.Studio__ "), "lm-studio");
        assert_eq!(slugify("über"), "ber");
    }

    #[test]
    fn titles() {
        assert_eq!(title_from_text("\n\n# Hello world\nmore"), "Hello world");
        assert_eq!(clean_title("\"Rust Borrow Checker Basics.\""), "Rust Borrow Checker Basics");
        assert_eq!(clean_title("Title: **Quick sort**"), "Quick sort");
        assert_eq!(title_from_text("   "), "New chat");
    }

    #[test]
    fn misc() {
        assert_eq!(short_model_name("openai/gpt-4o"), "gpt-4o");
        assert_eq!(short_model_name("llama3.2:3b"), "llama3.2:3b");
        assert_eq!(format_duration_ms(950), "950ms");
        assert_eq!(format_duration_ms(4200), "4.2s");
        assert_eq!(format_duration_ms(125_000), "2m 05s");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(sanitize_tool_name("mcp__my server__do.it"), "mcp__my_server__do_it");
        assert!(is_text_like("main.rs", "application/octet-stream"));
        assert!(!is_text_like("photo.png", "image/png"));
        assert_eq!(truncate_chars("abcdef", 4), "abc…");
    }
}
