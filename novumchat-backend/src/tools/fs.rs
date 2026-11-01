//! File tools: read, write, edit, list and search.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::{ToolOutput, truncate_output};
use crate::paths::{expand_tilde, normalize_lexically};

const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;
const DEFAULT_READ_LINES: usize = 2000;
const MAX_LINE_CHARS: usize = 2000;

/// Resolves a tool path against the workspace (supports `~` and relative paths).
pub fn resolve(workspace: &Path, raw: &str) -> PathBuf {
    let raw = raw.trim();
    let p = if raw.is_empty() || raw == "." { workspace.to_path_buf() } else { expand_tilde(raw) };
    let abs = if p.is_absolute() { p } else { workspace.join(p) };
    normalize_lexically(&abs)
}

/// Canonical form of `path`, following symlinks of the longest existing prefix.
fn canonical_prefix(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let mut base = dunce::canonicalize(&existing).unwrap_or(existing);
    for part in rest.into_iter().rev() {
        base.push(part);
    }
    base
}

/// Whether `path` lies inside `workspace` (after resolving symlinks).
pub fn inside(workspace: &Path, path: &Path) -> bool {
    let ws = canonical_prefix(workspace);
    canonical_prefix(path).starts_with(&ws)
}

/// Files that make other programs run code later: version-control internals
/// (hooks, config) and direnv's `.envrc`. Checked on the path as written and
/// after following symlinks.
pub fn sensitive(path: &Path) -> bool {
    let check = |p: &Path| {
        p.components().any(|c| matches!(c.as_os_str().to_str(), Some(".git" | ".hg" | ".svn")))
            || p.file_name().is_some_and(|n| n == ".envrc")
    };
    check(path) || check(&canonical_prefix(path))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    str_arg(args, key).ok_or_else(|| anyhow!("missing required argument `{key}`"))
}

fn usize_arg(args: &Value, key: &str) -> Option<usize> {
    args.get(key).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).map(|v| v as usize)
}

fn bool_arg(args: &Value, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))).unwrap_or(false)
}

pub async fn read_file(workspace: &Path, args: &Value, max_output: usize) -> Result<ToolOutput> {
    let path = resolve(workspace, req_str(args, "path")?);
    let meta = tokio::fs::metadata(&path).await.map_err(|e| anyhow!("{}: {e}", path.display()))?;
    if meta.is_dir() {
        bail!("{} is a directory; use list_dir", path.display());
    }
    if meta.len() > MAX_READ_BYTES {
        bail!("{} is too large ({} bytes); read a range with offset/limit after searching", path.display(), meta.len());
    }
    let bytes = tokio::fs::read(&path).await?;
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return Ok(ToolOutput::text(format!("{} is a binary file ({} bytes).", path.display(), bytes.len())));
    }
    let text = String::from_utf8_lossy(&bytes);
    let offset = usize_arg(args, "offset").unwrap_or(1).max(1);
    let limit = usize_arg(args, "limit").unwrap_or(DEFAULT_READ_LINES).clamp(1, 10_000);
    let total = text.lines().count();
    let mut out = String::new();
    for (i, line) in text.lines().enumerate().skip(offset - 1).take(limit) {
        let line = if line.chars().count() > MAX_LINE_CHARS {
            format!("{}… [line truncated]", line.chars().take(MAX_LINE_CHARS).collect::<String>())
        } else {
            line.to_string()
        };
        out.push_str(&format!("{:>6}\t{}\n", i + 1, line));
    }
    let shown_end = (offset - 1 + limit).min(total);
    if total == 0 {
        out.push_str("(empty file)\n");
    } else if shown_end < total || offset > 1 {
        out.push_str(&format!("\n[lines {}–{} of {}]", offset, shown_end, total));
    }
    Ok(ToolOutput::text(truncate_output(&out, max_output))
        .with_data(json!({ "path": path.display().to_string(), "lines": total })))
}

pub async fn write_file(workspace: &Path, args: &Value) -> Result<ToolOutput> {
    let path = resolve(workspace, req_str(args, "path")?);
    let content = req_str(args, "content")?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let existed = path.exists();
    tokio::fs::write(&path, content).await.map_err(|e| anyhow!("{}: {e}", path.display()))?;
    Ok(ToolOutput::text(format!(
        "{} {} ({} bytes, {} lines).",
        if existed { "Overwrote" } else { "Created" },
        path.display(),
        content.len(),
        content.lines().count()
    ))
    .with_data(json!({ "path": path.display().to_string() })))
}

/// Computes the edit without writing (also used for approval previews).
pub fn plan_edit(original: &str, old: &str, new: &str, replace_all: bool) -> Result<String> {
    if old.is_empty() {
        bail!("old_string must not be empty");
    }
    if old == new {
        bail!("old_string and new_string are identical");
    }
    let count = original.matches(old).count();
    match count {
        0 => bail!(
            "old_string was not found in the file. Read the file again and copy the exact text (including whitespace)."
        ),
        1 => Ok(original.replacen(old, new, 1)),
        _ if replace_all => Ok(original.replace(old, new)),
        n => bail!("old_string occurs {n} times. Add more surrounding context to make it unique, or set replace_all."),
    }
}

pub fn unified_diff(path: &str, before: &str, after: &str) -> String {
    let diff = similar::TextDiff::from_lines(before, after);
    diff.unified_diff().context_radius(3).header(&format!("a/{path}"), &format!("b/{path}")).to_string()
}

pub async fn edit_file(workspace: &Path, args: &Value, max_output: usize) -> Result<ToolOutput> {
    let path = resolve(workspace, req_str(args, "path")?);
    let old = req_str(args, "old_string")?;
    let new = str_arg(args, "new_string").unwrap_or("");
    let replace_all = bool_arg(args, "replace_all");
    let original = tokio::fs::read_to_string(&path).await.map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let updated = plan_edit(&original, old, new, replace_all)?;
    tokio::fs::write(&path, &updated).await?;
    let diff = unified_diff(&path.display().to_string(), &original, &updated);
    Ok(ToolOutput::text(format!("Edited {}.\n{}", path.display(), truncate_output(&diff, max_output)))
        .with_data(json!({ "path": path.display().to_string(), "diff": diff })))
}

pub async fn list_dir(workspace: &Path, args: &Value, max_output: usize) -> Result<ToolOutput> {
    let path = resolve(workspace, str_arg(args, "path").unwrap_or("."));
    let depth = usize_arg(args, "depth").unwrap_or(2).clamp(1, 6);
    let hidden = bool_arg(args, "show_hidden");
    let root = path.clone();
    let listing = tokio::task::spawn_blocking(move || -> Result<String> {
        if !root.is_dir() {
            bail!("{} is not a directory", root.display());
        }
        let mut out = format!("{}/\n", root.display());
        let mut count = 0usize;
        let walker = ignore::WalkBuilder::new(&root)
            .max_depth(Some(depth))
            .hidden(!hidden)
            .git_ignore(true)
            .sort_by_file_path(|a, b| a.cmp(b))
            .build();
        for entry in walker.flatten() {
            if entry.depth() == 0 {
                continue;
            }
            count += 1;
            if count > 800 {
                out.push_str("… (listing truncated)\n");
                break;
            }
            let indent = "  ".repeat(entry.depth() - 1);
            let name = entry.file_name().to_string_lossy();
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            if is_dir {
                out.push_str(&format!("{indent}{name}/\n"));
            } else {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                out.push_str(&format!("{indent}{name}  ({})\n", novumchat_core::util::human_bytes(size)));
            }
        }
        if count == 0 {
            out.push_str("(empty)\n");
        }
        Ok(out)
    })
    .await??;
    Ok(ToolOutput::text(truncate_output(&listing, max_output)))
}

pub async fn search_files(workspace: &Path, args: &Value, max_output: usize) -> Result<ToolOutput> {
    let pattern = req_str(args, "pattern")?.to_string();
    let root = resolve(workspace, str_arg(args, "path").unwrap_or("."));
    let glob = str_arg(args, "glob").map(String::from);
    let insensitive = bool_arg(args, "case_insensitive");
    let max_results = usize_arg(args, "max_results").unwrap_or(100).clamp(1, 500);
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let re =
            regex::RegexBuilder::new(&pattern).case_insensitive(insensitive).size_limit(1 << 22).build().or_else(
                |_| regex::RegexBuilder::new(&regex::escape(&pattern)).case_insensitive(insensitive).build(),
            )?;
        let mut builder = ignore::WalkBuilder::new(&root);
        builder.hidden(true).git_ignore(true);
        if let Some(g) = glob {
            let mut ob = ignore::overrides::OverrideBuilder::new(&root);
            ob.add(&g)?;
            builder.overrides(ob.build()?);
        }
        let mut out = String::new();
        let mut hits = 0usize;
        let mut files = 0usize;
        'walk: for entry in builder.build().flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            files += 1;
            if files > 20_000 {
                break;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() > 2 * 1024 * 1024 {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else { continue };
            if bytes.iter().take(4096).any(|&b| b == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path()).display().to_string();
            for (i, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    hits += 1;
                    let line = novumchat_core::util::truncate_chars(line.trim_end(), 300);
                    out.push_str(&format!("{rel}:{}: {line}\n", i + 1));
                    if hits >= max_results {
                        out.push_str(&format!("… stopped after {max_results} matches\n"));
                        break 'walk;
                    }
                }
            }
        }
        if hits == 0 {
            out = format!("No matches for `{pattern}` in {} ({files} files searched).", root.display());
        }
        Ok(out)
    })
    .await??;
    Ok(ToolOutput::text(truncate_output(&result, max_output)))
}

/// Paths a file tool will touch (for permission checks and approvals).
pub fn touched_paths(tool: &str, workspace: &Path, args: &Value) -> Vec<PathBuf> {
    match tool {
        "read_file" | "write_file" | "edit_file" => {
            str_arg(args, "path").map(|p| vec![resolve(workspace, p)]).unwrap_or_default()
        }
        "list_dir" | "search_files" => vec![resolve(workspace, str_arg(args, "path").unwrap_or("."))],
        _ => Vec::new(),
    }
}

/// Human preview of a write/edit for the approval dialog.
pub fn preview_change(tool: &str, workspace: &Path, args: &Value) -> String {
    let Some(raw) = str_arg(args, "path") else { return args.to_string() };
    let path = resolve(workspace, raw);
    let shown = path.display().to_string();
    match tool {
        "write_file" => {
            let content = str_arg(args, "content").unwrap_or("");
            match std::fs::read_to_string(&path) {
                Ok(before) => unified_diff(&shown, &before, content),
                Err(_) => {
                    let mut s = format!("New file {shown}\n\n");
                    for line in content.lines().take(80) {
                        s.push_str("+ ");
                        s.push_str(line);
                        s.push('\n');
                    }
                    if content.lines().count() > 80 {
                        s.push_str("…\n");
                    }
                    s
                }
            }
        }
        "edit_file" => {
            let old = str_arg(args, "old_string").unwrap_or("");
            let new = str_arg(args, "new_string").unwrap_or("");
            match std::fs::read_to_string(&path) {
                Ok(before) => match plan_edit(&before, old, new, bool_arg(args, "replace_all")) {
                    Ok(after) => unified_diff(&shown, &before, &after),
                    Err(e) => format!("{shown}\n(edit would fail: {e})"),
                },
                Err(e) => format!("{shown}\n({e})"),
            }
        }
        _ => shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_write_edit_list_search() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let out = write_file(ws, &json!({"path": "src/hello.txt", "content": "one\ntwo\nthree\n"})).await.unwrap();
        assert!(out.text.contains("Created"));
        let out = read_file(ws, &json!({"path": "src/hello.txt"}), 10_000).await.unwrap();
        assert!(out.text.contains("     2\ttwo"));
        let out = read_file(ws, &json!({"path": "src/hello.txt", "offset": 2, "limit": 1}), 10_000).await.unwrap();
        assert!(out.text.contains("two") && !out.text.contains("three\n"));
        let out = edit_file(ws, &json!({"path": "src/hello.txt", "old_string": "two", "new_string": "deux"}), 10_000)
            .await
            .unwrap();
        assert!(out.text.contains("+deux"), "{}", out.text);
        assert!(
            edit_file(ws, &json!({"path": "src/hello.txt", "old_string": "missing", "new_string": "x"}), 10_000)
                .await
                .is_err()
        );
        let out = list_dir(ws, &json!({}), 10_000).await.unwrap();
        assert!(out.text.contains("src/") && out.text.contains("hello.txt"), "{}", out.text);
        let out = search_files(ws, &json!({"pattern": "deu."}), 10_000).await.unwrap();
        assert!(out.text.contains("hello.txt:2: deux"), "{}", out.text);
    }

    #[test]
    fn workspace_containment() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        assert!(inside(ws, &resolve(ws, "a/b.txt")));
        assert!(!inside(ws, &resolve(ws, "../escape.txt")));
        assert!(!inside(ws, &resolve(ws, "/etc/passwd")));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", ws.join("link")).unwrap();
            assert!(!inside(ws, &resolve(ws, "link/passwd")));
        }
    }

    #[test]
    fn edits() {
        assert_eq!(plan_edit("a b a", "b", "c", false).unwrap(), "a c a");
        assert!(plan_edit("a b a", "a", "c", false).is_err());
        assert_eq!(plan_edit("a b a", "a", "c", true).unwrap(), "c b c");
    }
}
