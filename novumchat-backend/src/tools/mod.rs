//! Built-in agent tools and their permission assessment.

pub mod fs;
pub mod shell;
pub mod web;

use std::path::{Path, PathBuf};
use std::time::Duration;

use novumchat_core::config::{SearchConfig, ToolsConfig};
use novumchat_core::permissions::{CallContext, ToolRisk, command_allowlisted, command_denylisted};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::llm::ToolSpec;

#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    pub text: String,
    pub data: Option<Value>,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn text(s: impl Into<String>) -> Self {
        Self { text: s.into(), data: None, is_error: false }
    }

    pub fn error(s: impl Into<String>) -> Self {
        Self { text: s.into(), data: None, is_error: true }
    }

    pub fn with_data(mut self, v: Value) -> Self {
        self.data = Some(v);
        self
    }
}

/// Keeps the head and tail of long output.
pub fn truncate_output(s: &str, max: usize) -> String {
    if s.len() <= max || max < 200 {
        return s.to_string();
    }
    let head_len = max * 7 / 10;
    let tail_len = max - head_len;
    let mut head_end = head_len;
    while !s.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = s.len() - tail_len;
    while !s.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    format!("{}\n\n… [{} bytes omitted] …\n\n{}", &s[..head_end], tail_start - head_end, &s[tail_start..])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Builtin {
    Bash,
    ReadFile,
    WriteFile,
    EditFile,
    ListDir,
    SearchFiles,
    WebSearch,
    FetchUrl,
}

impl Builtin {
    pub const ALL: [Builtin; 8] = [
        Builtin::Bash,
        Builtin::ReadFile,
        Builtin::WriteFile,
        Builtin::EditFile,
        Builtin::ListDir,
        Builtin::SearchFiles,
        Builtin::WebSearch,
        Builtin::FetchUrl,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Builtin::Bash => "bash",
            Builtin::ReadFile => "read_file",
            Builtin::WriteFile => "write_file",
            Builtin::EditFile => "edit_file",
            Builtin::ListDir => "list_dir",
            Builtin::SearchFiles => "search_files",
            Builtin::WebSearch => "web_search",
            Builtin::FetchUrl => "fetch_url",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.name() == name)
    }

    pub fn risk(self) -> ToolRisk {
        match self {
            Builtin::Bash => ToolRisk::Execute,
            Builtin::ReadFile | Builtin::ListDir | Builtin::SearchFiles => ToolRisk::Read,
            Builtin::WriteFile | Builtin::EditFile => ToolRisk::Write,
            Builtin::WebSearch => ToolRisk::Search,
            Builtin::FetchUrl => ToolRisk::Network,
        }
    }

    /// Web tools are controlled by the chat's "web search" toggle.
    pub fn is_web(self) -> bool {
        matches!(self, Builtin::WebSearch | Builtin::FetchUrl)
    }

    pub fn description(self) -> &'static str {
        match self {
            Builtin::Bash => {
                "Run a shell command (bash on Linux/macOS, PowerShell on Windows) in the workspace directory and \
return its combined stdout/stderr and exit code. Use for builds, tests, git, package managers and file \
operations not covered by other tools. Commands run non-interactively with a timeout; avoid commands that wait \
for input. Prefer read_file/edit_file for reading and changing files."
            }
            Builtin::ReadFile => {
                "Read a text file. Returns numbered lines (`  line\\tcontent`). Paths are relative to the workspace \
unless absolute. Use offset/limit for large files."
            }
            Builtin::WriteFile => {
                "Create or overwrite a file with the given content (parent directories are created). Prefer \
edit_file for changing part of an existing file."
            }
            Builtin::EditFile => {
                "Replace an exact string in a file. old_string must match the file content exactly (including \
indentation) and be unique unless replace_all is true. Read the file first."
            }
            Builtin::ListDir => "List files and folders (recursively up to `depth`, honouring .gitignore).",
            Builtin::SearchFiles => {
                "Search file contents with a regular expression (like ripgrep). Returns `path:line: text` matches. \
Optionally restrict files with a glob such as `*.rs`."
            }
            Builtin::WebSearch => {
                "Search the internet. Returns titles, URLs and snippets. Use for current events, facts you are \
unsure of, documentation and anything after your knowledge cutoff. Cite results as [n](url)."
            }
            Builtin::FetchUrl => {
                "Download a web page or file by URL and return its readable text content (HTML is converted to \
text). Use after web_search to read a source in detail."
            }
        }
    }

    pub fn parameters(self) -> Value {
        match self {
            Builtin::Bash => json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command line to execute" },
                    "cwd": { "type": "string", "description": "Working directory, relative to the workspace (optional)" },
                    "timeout_secs": { "type": "integer", "description": "Timeout in seconds (optional)" }
                },
                "required": ["command"]
            }),
            Builtin::ReadFile => json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" },
                    "offset": { "type": "integer", "description": "First line to read (1-based)" },
                    "limit": { "type": "integer", "description": "Maximum number of lines" }
                },
                "required": ["path"]
            }),
            Builtin::WriteFile => json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" },
                    "content": { "type": "string", "description": "Complete new file content" }
                },
                "required": ["path", "content"]
            }),
            Builtin::EditFile => json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" },
                    "old_string": { "type": "string", "description": "Exact text to replace" },
                    "new_string": { "type": "string", "description": "Replacement text" },
                    "replace_all": { "type": "boolean", "description": "Replace every occurrence" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
            Builtin::ListDir => json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory (default: workspace root)" },
                    "depth": { "type": "integer", "description": "Recursion depth 1-6 (default 2)" },
                    "show_hidden": { "type": "boolean", "description": "Include dotfiles" }
                }
            }),
            Builtin::SearchFiles => json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regular expression" },
                    "path": { "type": "string", "description": "Directory to search (default: workspace)" },
                    "glob": { "type": "string", "description": "File filter such as *.py" },
                    "case_insensitive": { "type": "boolean" },
                    "max_results": { "type": "integer" }
                },
                "required": ["pattern"]
            }),
            Builtin::WebSearch => json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "max_results": { "type": "integer", "description": "Number of results (1-20)" }
                },
                "required": ["query"]
            }),
            Builtin::FetchUrl => json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "http(s) URL" },
                    "max_chars": { "type": "integer", "description": "Maximum characters to return" }
                },
                "required": ["url"]
            }),
        }
    }

    pub fn spec(self) -> ToolSpec {
        ToolSpec { name: self.name().into(), description: self.description().into(), parameters: self.parameters() }
    }
}

pub struct ToolEnv<'a> {
    pub workspace: &'a Path,
    pub tools: &'a ToolsConfig,
    pub search: &'a SearchConfig,
    pub cancel: &'a CancellationToken,
}

/// Permission facts plus approval texts for one call.
pub struct Assessment {
    pub ctx: CallContext,
    pub title: String,
    pub detail: String,
}

fn arg_str<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn assess_builtin(tool: Builtin, args: &Value, workspace: &Path, cfg: &ToolsConfig) -> Assessment {
    let mut ctx = CallContext::new(tool.risk());
    let paths = fs::touched_paths(tool.name(), workspace, args);
    ctx.inside_workspace = paths.iter().all(|p| fs::inside(workspace, p));
    ctx.sensitive = paths.iter().any(|p| fs::sensitive(p));
    let (title, detail) = match tool {
        Builtin::Bash => {
            let command = arg_str(args, "command");
            let cwd = args.get("cwd").and_then(Value::as_str).map(|c| fs::resolve(workspace, c));
            if let Some(c) = &cwd {
                ctx.inside_workspace = fs::inside(workspace, c);
            }
            let where_ = cwd.unwrap_or_else(|| workspace.to_path_buf());
            ctx.allowlisted =
                command_allowlisted(command, &cfg.allowed_commands) && args_stay_inside(command, &where_, workspace);
            ctx.denylisted = command_denylisted(command, &cfg.denied_commands);
            ("Run a shell command".to_string(), format!("$ {command}\n\n(in {})", where_.display()))
        }
        Builtin::ReadFile | Builtin::ListDir | Builtin::SearchFiles => {
            let what = paths.first().map(|p| p.display().to_string()).unwrap_or_default();
            let title = if ctx.inside_workspace { "Read files" } else { "Read outside the workspace" };
            let detail = if tool == Builtin::SearchFiles {
                format!("Search `{}` in {what}", arg_str(args, "pattern"))
            } else {
                what
            };
            (title.to_string(), detail)
        }
        Builtin::WriteFile | Builtin::EditFile => {
            let title = match (tool, ctx.inside_workspace) {
                _ if ctx.sensitive => "Change a file that can run commands",
                (Builtin::WriteFile, true) => "Write a file",
                (Builtin::WriteFile, false) => "Write a file outside the workspace",
                (_, true) => "Edit a file",
                (_, false) => "Edit a file outside the workspace",
            };
            (title.to_string(), fs::preview_change(tool.name(), workspace, args))
        }
        Builtin::WebSearch => ("Search the web".to_string(), arg_str(args, "query").to_string()),
        Builtin::FetchUrl => ("Fetch a web page".to_string(), arg_str(args, "url").to_string()),
    };
    Assessment { ctx, title, detail }
}

/// Whether every argument of an allow-listed command that names an existing
/// file (directly, or through a glob) resolves inside the workspace, so a
/// symlink cannot lead a command like `cat` elsewhere without asking.
fn args_stay_inside(command: &str, base: &Path, workspace: &Path) -> bool {
    let words = command.split_whitespace().map(|w| w.replace(['"', '\'', '\\'], ""));
    let candidates = words.flat_map(|w| {
        let value = w.split_once('=').map(|(_, v)| v.to_string());
        [Some(w), value].into_iter().flatten()
    });
    candidates.filter(|w| !w.is_empty() && !w.starts_with('-')).all(|w| {
        let parts: Vec<&str> = w.split('/').collect();
        let glob_at = parts.iter().position(|p| p.contains(['*', '?', '[']));
        let fixed: PathBuf = parts[..glob_at.unwrap_or(parts.len())].iter().collect();
        let dir = base.join(&fixed);
        if dir.symlink_metadata().is_ok() && !fs::inside(workspace, &dir) {
            return false;
        }
        // The glob may match a symlink in that directory.
        let Some(at) = glob_at else { return true };
        let matcher = globset::Glob::new(parts[at]).ok().map(|g| g.compile_matcher());
        std::fs::read_dir(&dir).map_or(true, |entries| {
            entries.flatten().all(|e| {
                let matched = matcher.as_ref().is_none_or(|m| m.is_match(e.file_name()));
                !matched || !e.file_type().is_ok_and(|t| t.is_symlink()) || fs::inside(workspace, &e.path())
            })
        })
    })
}

/// Executes a built-in tool. Failures become error outputs for the model.
pub async fn run_builtin(tool: Builtin, args: &Value, env: &ToolEnv<'_>) -> ToolOutput {
    let max = env.tools.max_output_bytes.max(1000);
    let result = match tool {
        Builtin::Bash => {
            let command = arg_str(args, "command");
            let cwd = args
                .get("cwd")
                .and_then(Value::as_str)
                .map(|c| fs::resolve(env.workspace, c))
                .unwrap_or_else(|| env.workspace.to_path_buf());
            let secs = args
                .get("timeout_secs")
                .and_then(Value::as_u64)
                .unwrap_or(env.tools.bash_timeout_secs)
                .clamp(1, env.tools.bash_timeout_secs.max(1) * 4);
            shell::run_command(command, &cwd, Duration::from_secs(secs), max, env.tools.shell.as_deref(), env.cancel)
                .await
        }
        Builtin::ReadFile => fs::read_file(env.workspace, args, max).await,
        Builtin::WriteFile => fs::write_file(env.workspace, args).await,
        Builtin::EditFile => fs::edit_file(env.workspace, args, max).await,
        Builtin::ListDir => fs::list_dir(env.workspace, args, max).await,
        Builtin::SearchFiles => fs::search_files(env.workspace, args, max).await,
        Builtin::WebSearch => {
            let query = arg_str(args, "query");
            let n = args.get("max_results").and_then(Value::as_u64).map(|n| n as usize);
            crate::search::search(env.search, query, n).await.map(|resp| {
                ToolOutput::text(crate::search::format_for_model(&resp))
                    .with_data(serde_json::to_value(&resp).unwrap_or(Value::Null))
            })
        }
        Builtin::FetchUrl => web::fetch_url(args, max).await,
    };
    result.unwrap_or_else(|e| ToolOutput::error(format!("Error: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_keeps_head_and_tail() {
        let s = "a".repeat(1000) + &"b".repeat(1000);
        let t = truncate_output(&s, 500);
        assert!(t.starts_with("aaa"));
        assert!(t.ends_with("bbb"));
        assert!(t.contains("bytes omitted"));
        assert_eq!(truncate_output("short", 500), "short");
    }

    #[test]
    fn assessments() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ToolsConfig::default();
        let a = assess_builtin(Builtin::Bash, &json!({"command": "ls -la"}), dir.path(), &cfg);
        assert!(a.ctx.allowlisted && !a.ctx.denylisted);
        let a = assess_builtin(Builtin::Bash, &json!({"command": "rm -rf /"}), dir.path(), &cfg);
        assert!(a.ctx.denylisted);
        let a = assess_builtin(Builtin::ReadFile, &json!({"path": "/etc/hostname"}), dir.path(), &cfg);
        assert!(!a.ctx.inside_workspace);
        let a = assess_builtin(Builtin::WriteFile, &json!({"path": "x.txt", "content": "hi"}), dir.path(), &cfg);
        assert!(a.ctx.inside_workspace);
        assert!(a.detail.contains("+ hi"));
        for b in Builtin::ALL {
            assert_eq!(Builtin::from_name(b.name()), Some(b));
            assert_eq!(b.parameters()["type"], "object");
        }
    }

    #[test]
    fn hooks_and_config_in_git_are_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ToolsConfig::default();
        let a = assess_builtin(Builtin::WriteFile, &json!({"path": ".git/config", "content": "x"}), dir.path(), &cfg);
        assert!(a.ctx.inside_workspace && a.ctx.sensitive);
        let a = assess_builtin(Builtin::EditFile, &json!({"path": "repo/.git/hooks/pre-commit"}), dir.path(), &cfg);
        assert!(a.ctx.sensitive);
        let a = assess_builtin(Builtin::WriteFile, &json!({"path": ".envrc", "content": "x"}), dir.path(), &cfg);
        assert!(a.ctx.sensitive);
        let a = assess_builtin(Builtin::WriteFile, &json!({"path": "src/git.rs", "content": "x"}), dir.path(), &cfg);
        assert!(!a.ctx.sensitive);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_lead_allowlisted_commands_outside() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "s3cret").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "hello").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("elsewhere")).unwrap();
        let cfg = ToolsConfig::default();
        let allowed = |c: &str| assess_builtin(Builtin::Bash, &json!({"command": c}), dir.path(), &cfg).ctx.allowlisted;
        assert!(allowed("cat notes.txt"));
        assert!(allowed("cat *.txt"));
        assert!(!allowed("cat elsewhere/secret.txt"));
        assert!(!allowed("cat elsewhere/*"));
        // `*` in the workspace matches the symlink itself.
        assert!(!allowed("grep -r s3cret *"));
    }
}
