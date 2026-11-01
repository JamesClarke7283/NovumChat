//! Tool permission model.
//!
//! Every tool call is classified by [`ToolRisk`]; the active
//! [`PermissionLevel`] then decides whether it runs, asks the user or is
//! refused. The defaults are deliberately conservative: anything that changes
//! files, runs commands or talks to third-party MCP servers asks first.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    /// Only reading files and searching the web are possible.
    ReadOnly,
    /// Reads run automatically; edits, commands and MCP tools ask first.
    #[default]
    Ask,
    /// Like `Ask`, but file edits inside the workspace run without asking.
    AutoEdit,
    /// Trust everything. Only `denied_commands` are still refused.
    Yolo,
}

impl PermissionLevel {
    pub const ALL: [PermissionLevel; 4] =
        [PermissionLevel::ReadOnly, PermissionLevel::Ask, PermissionLevel::AutoEdit, PermissionLevel::Yolo];

    pub fn as_str(self) -> &'static str {
        match self {
            PermissionLevel::ReadOnly => "read_only",
            PermissionLevel::Ask => "ask",
            PermissionLevel::AutoEdit => "auto_edit",
            PermissionLevel::Yolo => "yolo",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            PermissionLevel::ReadOnly => "Read only",
            PermissionLevel::Ask => "Ask before changes",
            PermissionLevel::AutoEdit => "Auto-edit workspace",
            PermissionLevel::Yolo => "YOLO",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            PermissionLevel::ReadOnly => {
                "Models can read files in the workspace and search the web. Nothing can be changed or executed."
            }
            PermissionLevel::Ask => {
                "Reading is automatic. Editing files, running commands, fetching pages and MCP tools ask for your approval."
            }
            PermissionLevel::AutoEdit => {
                "File edits inside the workspace happen automatically. Commands and MCP tools still ask."
            }
            PermissionLevel::Yolo => {
                "Everything runs without asking, anywhere on your system. Only denied commands are blocked. Use with care."
            }
        }
    }
}

/// What a tool call can do.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ToolRisk {
    /// Read files or list directories.
    Read,
    /// Query the configured search engine.
    Search,
    /// Download an arbitrary URL.
    Network,
    /// Create, modify or delete files.
    Write,
    /// Run shell commands.
    Execute,
    /// Tools provided by an MCP server.
    External,
}

impl ToolRisk {
    pub fn label(self) -> &'static str {
        match self {
            ToolRisk::Read => "Read",
            ToolRisk::Search => "Search",
            ToolRisk::Network => "Network",
            ToolRisk::Write => "Write",
            ToolRisk::Execute => "Execute",
            ToolRisk::External => "MCP",
        }
    }
}

/// Outcome of the permission policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allow,
    Ask,
    Deny,
}

/// Facts about one concrete tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallContext {
    pub risk: ToolRisk,
    /// All paths touched by the call are inside the workspace.
    pub inside_workspace: bool,
    /// Shell command matches `allowed_commands` (and contains no chaining).
    pub allowlisted: bool,
    /// Shell command matches `denied_commands`.
    pub denylisted: bool,
    /// The user chose "always allow" for this tool in this chat.
    pub granted_for_chat: bool,
    /// MCP server marked `trusted = true`.
    pub trusted_server: bool,
    /// MCP tool annotated as read-only.
    pub read_only_hint: bool,
    /// Touches a file that makes other programs run code later (`.git/`
    /// internals such as hooks and config, `.envrc`). Writes to these always
    /// ask below YOLO, so an auto-approved edit cannot plant a command that an
    /// allow-listed one (like `git status`) would then run.
    pub sensitive: bool,
}

impl CallContext {
    pub fn new(risk: ToolRisk) -> Self {
        Self {
            risk,
            inside_workspace: true,
            allowlisted: false,
            denylisted: false,
            granted_for_chat: false,
            trusted_server: false,
            read_only_hint: false,
            sensitive: false,
        }
    }
}

/// Should a tool with this risk be offered to the model at all?
pub fn tool_offered(level: PermissionLevel, risk: ToolRisk) -> bool {
    match level {
        PermissionLevel::ReadOnly => {
            matches!(risk, ToolRisk::Read | ToolRisk::Search | ToolRisk::Network | ToolRisk::External)
        }
        _ => true,
    }
}

/// The permission policy.
pub fn decide(level: PermissionLevel, ctx: CallContext) -> Verdict {
    if ctx.denylisted {
        return Verdict::Deny;
    }
    if level == PermissionLevel::Yolo {
        return Verdict::Allow;
    }
    if level == PermissionLevel::ReadOnly {
        return match ctx.risk {
            ToolRisk::Read if ctx.inside_workspace => Verdict::Allow,
            ToolRisk::Search => Verdict::Allow,
            ToolRisk::Read | ToolRisk::Network => {
                if ctx.granted_for_chat {
                    Verdict::Allow
                } else {
                    Verdict::Ask
                }
            }
            ToolRisk::External if ctx.read_only_hint => {
                if ctx.granted_for_chat || ctx.trusted_server {
                    Verdict::Allow
                } else {
                    Verdict::Ask
                }
            }
            _ => Verdict::Deny,
        };
    }
    if ctx.risk == ToolRisk::Write && ctx.sensitive {
        return Verdict::Ask;
    }
    if ctx.granted_for_chat {
        return Verdict::Allow;
    }
    match ctx.risk {
        ToolRisk::Read => {
            if ctx.inside_workspace {
                Verdict::Allow
            } else {
                Verdict::Ask
            }
        }
        ToolRisk::Search => Verdict::Allow,
        ToolRisk::Network => {
            if level == PermissionLevel::AutoEdit {
                Verdict::Allow
            } else {
                Verdict::Ask
            }
        }
        ToolRisk::Write => {
            if level == PermissionLevel::AutoEdit && ctx.inside_workspace {
                Verdict::Allow
            } else {
                Verdict::Ask
            }
        }
        ToolRisk::Execute => {
            if ctx.allowlisted && ctx.inside_workspace {
                Verdict::Allow
            } else {
                Verdict::Ask
            }
        }
        ToolRisk::External => {
            if ctx.trusted_server {
                Verdict::Allow
            } else {
                Verdict::Ask
            }
        }
    }
}

/// Characters that chain, redirect or expand shell commands. A command
/// containing any of them is never treated as allow-listed.
const SHELL_META: &[&str] = &["&&", "||", ";", "|", "`", "$", ">", "<", "\n", "\r", "&", "~"];

/// Whether `command` starts with one of the allow-listed prefixes and can be
/// run without asking: it must not chain or expand anything, must stay inside
/// the working directory (no absolute paths, `..` or dot-globs that reach the
/// parent), and must not use an option that makes a read-only program write
/// files, run other programs or follow links (`find -delete`, `rg --pre`,
/// `git branch -D`, …). Anything else falls back to asking.
pub fn command_allowlisted(command: &str, allowed: &[String]) -> bool {
    let cmd = command.trim();
    if cmd.is_empty() || SHELL_META.iter().any(|m| cmd.contains(m)) {
        return false;
    }
    let prefixed = allowed.iter().any(|prefix| {
        let prefix = prefix.trim();
        !prefix.is_empty()
            && (cmd == prefix || cmd.strip_prefix(prefix).is_some_and(|rest| rest.starts_with(char::is_whitespace)))
    });
    if !prefixed {
        return false;
    }
    // The shell removes quotes and backslashes before the program sees its
    // arguments, so judge the arguments without them too.
    let words: Vec<String> = cmd.split_whitespace().map(|w| w.replace(['"', '\'', '\\'], "")).collect();
    let Some((program, args)) = words.split_first() else { return false };
    if args.iter().any(|a| escapes_cwd(a) || a.split_once('=').is_some_and(|(_, v)| escapes_cwd(v))) {
        return false;
    }
    if program == "git" && args.first().is_some_and(|sub| sub == "branch") && !git_branch_lists_only(&args[1..]) {
        return false;
    }
    let (words_risky, letters_risky) = risky_options(program);
    !args.iter().any(|a| {
        words_risky.iter().any(|w| a.starts_with(w))
            || (a.len() > 1
                && a.starts_with('-')
                && !a.starts_with("--")
                && a[1..].chars().any(|c| letters_risky.contains(&c)))
    })
}

/// Whether a shell word points outside the working directory.
fn escapes_cwd(word: &str) -> bool {
    let bytes = word.as_bytes();
    let windows_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    word.starts_with('/')
        || windows_drive
        || word.split('/').any(|part| part == ".." || (part.starts_with('.') && part.contains(['*', '?', '['])))
}

/// Options that make an otherwise read-only program write files, run other
/// programs or follow symbolic links: (option prefixes, short option letters).
fn risky_options(program: &str) -> (&'static [&'static str], &'static [char]) {
    match program {
        "find" => (&["-delete", "-exec", "-ok", "-fprint", "-fls", "-follow", "-L"], &[]),
        "rg" => (&["--pre", "--hostname-bin", "--follow"], &['L']),
        "grep" => (&["--dereference-recursive"], &['R']),
        "tree" => (&[], &['o', 'R', 'l']),
        "du" => (&["--dereference"], &['L']),
        "date" => (&["--set"], &['s']),
        "file" => (&["--compile"], &['C']),
        "git" => (&["--output", "--ext-diff"], &[]),
        _ => (&[], &[]),
    }
}

/// `git branch` only lists branches when given listing options; a bare name
/// creates a branch and `-d`/`-m`/`-c` delete, rename or copy one.
fn git_branch_lists_only(args: &[String]) -> bool {
    const LISTING: &[&str] = &[
        "-a",
        "--all",
        "-r",
        "--remotes",
        "-v",
        "-vv",
        "--verbose",
        "-l",
        "--list",
        "--show-current",
        "-i",
        "--ignore-case",
        "--color",
        "--no-color",
        "--column",
        "--no-column",
        "--no-abbrev",
    ];
    const LISTING_WITH_VALUE: &[&str] = &[
        "--sort=",
        "--format=",
        "--contains",
        "--no-contains",
        "--merged",
        "--no-merged",
        "--points-at",
        "--color=",
        "--column=",
        "--abbrev=",
    ];
    let listing = args.iter().any(|a| a == "-l" || a == "--list");
    args.iter().all(|a| {
        if a.starts_with('-') {
            LISTING.contains(&a.as_str()) || LISTING_WITH_VALUE.iter().any(|p| a.starts_with(p))
        } else {
            // Patterns after --list; anything else would be a new branch name.
            listing
        }
    })
}

/// Whether `command` contains a denied pattern (whitespace-insensitive).
///
/// A pattern only matches on token boundaries, so `rm -rf /` blocks
/// `rm -rf /` and `rm -rf / --no-preserve-root` but not `rm -rf /tmp/build`,
/// while `mkfs` also blocks `mkfs.ext4`.
pub fn command_denylisted(command: &str, denied: &[String]) -> bool {
    let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let cmd = squash(command);
    denied.iter().any(|pattern| {
        let p = squash(pattern);
        if p.is_empty() {
            return false;
        }
        let ends_alnum = p.chars().last().is_some_and(|c| c.is_alphanumeric());
        cmd.match_indices(&p).any(|(start, _)| {
            let before_ok = cmd[..start].chars().last().is_none_or(|c| !c.is_alphanumeric());
            let after = cmd[start + p.len()..].chars().next();
            let after_ok = match after {
                None => true,
                Some(c) if c.is_whitespace() || ";&|)".contains(c) => true,
                Some(c) => ends_alnum && !c.is_alphanumeric(),
            };
            before_ok && after_ok
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(risk: ToolRisk) -> CallContext {
        CallContext::new(risk)
    }

    #[test]
    fn safe_defaults_ask_for_changes() {
        let l = PermissionLevel::default();
        assert_eq!(l, PermissionLevel::Ask);
        assert_eq!(decide(l, ctx(ToolRisk::Read)), Verdict::Allow);
        assert_eq!(decide(l, ctx(ToolRisk::Search)), Verdict::Allow);
        assert_eq!(decide(l, ctx(ToolRisk::Network)), Verdict::Ask);
        assert_eq!(decide(l, ctx(ToolRisk::Write)), Verdict::Ask);
        assert_eq!(decide(l, ctx(ToolRisk::Execute)), Verdict::Ask);
        assert_eq!(decide(l, ctx(ToolRisk::External)), Verdict::Ask);
        let outside = CallContext { inside_workspace: false, ..ctx(ToolRisk::Read) };
        assert_eq!(decide(l, outside), Verdict::Ask);
    }

    #[test]
    fn read_only_denies_changes() {
        let l = PermissionLevel::ReadOnly;
        assert_eq!(decide(l, ctx(ToolRisk::Write)), Verdict::Deny);
        assert_eq!(decide(l, ctx(ToolRisk::Execute)), Verdict::Deny);
        assert_eq!(decide(l, ctx(ToolRisk::External)), Verdict::Deny);
        let ro_mcp = CallContext { read_only_hint: true, ..ctx(ToolRisk::External) };
        assert_eq!(decide(l, ro_mcp), Verdict::Ask);
        assert!(!tool_offered(l, ToolRisk::Execute));
        assert!(tool_offered(l, ToolRisk::Read));
    }

    #[test]
    fn auto_edit_and_yolo() {
        let l = PermissionLevel::AutoEdit;
        assert_eq!(decide(l, ctx(ToolRisk::Write)), Verdict::Allow);
        let outside = CallContext { inside_workspace: false, ..ctx(ToolRisk::Write) };
        assert_eq!(decide(l, outside), Verdict::Ask);
        assert_eq!(decide(l, ctx(ToolRisk::Execute)), Verdict::Ask);
        let y = PermissionLevel::Yolo;
        assert_eq!(decide(y, outside), Verdict::Allow);
        assert_eq!(decide(y, ctx(ToolRisk::Execute)), Verdict::Allow);
        let denied = CallContext { denylisted: true, ..ctx(ToolRisk::Execute) };
        assert_eq!(decide(y, denied), Verdict::Deny);
        // Files that make other tools run code always ask below YOLO.
        let hook = CallContext { sensitive: true, granted_for_chat: true, ..ctx(ToolRisk::Write) };
        assert_eq!(decide(l, hook), Verdict::Ask);
        assert_eq!(decide(PermissionLevel::Ask, hook), Verdict::Ask);
        assert_eq!(decide(y, hook), Verdict::Allow);
    }

    #[test]
    fn grants_and_allowlists() {
        let l = PermissionLevel::Ask;
        let granted = CallContext { granted_for_chat: true, ..ctx(ToolRisk::Execute) };
        assert_eq!(decide(l, granted), Verdict::Allow);
        let listed = CallContext { allowlisted: true, ..ctx(ToolRisk::Execute) };
        assert_eq!(decide(l, listed), Verdict::Allow);
        let allowed = vec!["ls".to_string(), "git status".to_string()];
        assert!(command_allowlisted("ls -la", &allowed));
        assert!(command_allowlisted("git status", &allowed));
        assert!(!command_allowlisted("lsblk", &allowed));
        assert!(!command_allowlisted("ls; rm -rf x", &allowed));
        assert!(!command_allowlisted("ls | sh", &allowed));
        assert!(!command_allowlisted("git status && curl x", &allowed));
    }

    #[test]
    fn allowlisted_commands_stay_read_only_and_local() {
        let allowed: Vec<String> =
            ["ls", "cat", "find", "rg", "grep", "tree", "git status", "git log", "git branch", "date"]
                .into_iter()
                .map(String::from)
                .collect();
        let ok = |c: &str| command_allowlisted(c, &allowed);
        // Ordinary read-only use runs without asking.
        assert!(ok("ls -la src"));
        assert!(ok("grep -rn \"TODO\" src"));
        assert!(ok("rg -n 'fn main' --glob '*.rs'"));
        assert!(ok("find . -name '*.rs' -type f"));
        assert!(ok("cat src/*.rs"));
        assert!(ok("git log --oneline -n 5"));
        assert!(ok("git branch -a"));
        assert!(ok("git branch --list 'feat/*'"));
        assert!(ok("tree -L 2"));
        // Leaving the working directory asks.
        assert!(!ok("cat /etc/passwd"));
        assert!(!ok("cat ../secrets.txt"));
        assert!(!ok("cat ~/.ssh/id_rsa"));
        assert!(!ok("cat $HOME/.ssh/id_rsa"));
        assert!(!ok("cat ${HOME}/.netrc"));
        assert!(!ok("grep -r password .*"));
        assert!(!ok("cat .?/.bashrc"));
        assert!(!ok("rg --files-from=/etc/hosts"));
        assert!(!ok("cat C:/Windows/win.ini"));
        // Options that write, execute or follow links ask.
        assert!(!ok("find . -delete"));
        assert!(!ok("find . -exec rm {} +"));
        assert!(!ok("find . -execdir sh -c x {} +"));
        assert!(!ok("find . -fprint out.txt"));
        assert!(!ok("find . -de'lete'"));
        assert!(!ok("rg --pre ./run.sh x"));
        assert!(!ok("rg --pre=./run.sh x"));
        assert!(!ok("rg -L secret"));
        assert!(!ok("grep -Rn x ."));
        assert!(!ok("tree -o out.txt"));
        assert!(!ok("git log --output=notes.txt"));
        assert!(!ok("git branch -D main"));
        assert!(!ok("git branch new-feature"));
        assert!(!ok("git branch -m old new"));
        assert!(!ok("date -s 2020-01-01"));
        let denied: Vec<String> = ["rm -rf /", "mkfs", "shutdown"].into_iter().map(String::from).collect();
        assert!(command_denylisted("sudo rm   -rf /", &denied));
        assert!(command_denylisted("rm -rf / --no-preserve-root", &denied));
        assert!(!command_denylisted("rm -rf ./build", &denied));
        assert!(!command_denylisted("rm -rf /tmp/build", &denied));
        assert!(command_denylisted("mkfs.ext4 /dev/sda1", &denied));
        assert!(command_denylisted("echo hi; shutdown now", &denied));
        assert!(!command_denylisted("echo noshutdown", &denied));
    }
}
