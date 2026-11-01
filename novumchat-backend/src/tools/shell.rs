//! The `bash` tool: runs shell commands in the workspace.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use super::{ToolOutput, truncate_output};

/// Can this platform spawn processes at all?
pub fn shell_available() -> bool {
    !cfg!(target_os = "ios")
}

fn which(program: &str) -> Option<PathBuf> {
    let path = augmented_path();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        for ext in ["exe", "cmd", "bat"] {
            let c = dir.join(format!("{program}.{ext}"));
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

/// The shell program and the flag that introduces a command string.
pub fn shell_program(configured: Option<&str>) -> (String, Vec<String>) {
    if let Some(s) = configured.map(str::trim).filter(|s| !s.is_empty()) {
        let lower = s.to_lowercase();
        if lower.contains("powershell") || lower.contains("pwsh") {
            return (s.into(), vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into()]);
        }
        if lower.ends_with("cmd") || lower.ends_with("cmd.exe") {
            return (s.into(), vec!["/C".into()]);
        }
        return (s.into(), vec!["-c".into()]);
    }
    if cfg!(windows) {
        if which("pwsh").is_some() {
            return ("pwsh".into(), vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into()]);
        }
        return ("powershell".into(), vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into()]);
    }
    if which("bash").is_some() {
        ("bash".into(), vec!["-c".into()])
    } else if Path::new("/system/bin/sh").exists() {
        ("/system/bin/sh".into(), vec!["-c".into()])
    } else {
        ("sh".into(), vec!["-c".into()])
    }
}

/// `PATH` plus common tool locations that GUI launches often miss
/// (Homebrew, ~/.local/bin, cargo, bun, nvm…).
pub fn augmented_path() -> OsString {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<PathBuf> = std::env::split_paths(&current).collect();
    let mut extra: Vec<PathBuf> = ["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"]
        .iter()
        .map(PathBuf::from)
        .collect();
    if let Some(base) = directories::BaseDirs::new() {
        let home = base.home_dir();
        for rel in [".local/bin", ".cargo/bin", ".bun/bin", ".deno/bin", "go/bin", ".volta/bin", ".npm-global/bin"] {
            extra.push(home.join(rel));
        }
        // Newest nvm-managed node.
        if let Ok(rd) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            let mut versions: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("bin")).collect();
            versions.sort();
            if let Some(last) = versions.pop() {
                extra.push(last);
            }
        }
    }
    for p in extra {
        if p.is_dir() && !paths.contains(&p) {
            paths.push(p);
        }
    }
    std::env::join_paths(paths).unwrap_or(current)
}

async fn read_capped<R: tokio::io::AsyncRead + Unpin>(mut r: R, cap: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if out.len() < cap {
                    let take = n.min(cap - out.len());
                    out.extend_from_slice(&buf[..take]);
                }
            }
        }
    }
    out
}

#[cfg(unix)]
fn kill_tree(pid: Option<u32>) {
    if let Some(pid) = pid {
        // The child leads its own process group: kill the whole group.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_tree(_pid: Option<u32>) {}

pub async fn run_command(
    command: &str,
    cwd: &Path,
    timeout: Duration,
    max_output: usize,
    shell: Option<&str>,
    cancel: &CancellationToken,
) -> Result<ToolOutput> {
    if !shell_available() {
        return Err(anyhow!("Running commands is not supported on this platform."));
    }
    if command.trim().is_empty() {
        return Err(anyhow!("missing required argument `command`"));
    }
    tokio::fs::create_dir_all(cwd).await.ok();
    let (program, flags) = shell_program(shell);
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&flags)
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("PATH", augmented_path())
        .env("TERM", "dumb")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .env("NOVUMCHAT", "1");
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let started = Instant::now();
    let mut child = cmd.spawn().map_err(|e| anyhow!("failed to start {program}: {e}"))?;
    let pid = child.id();
    let cap = 8 * 1024 * 1024;
    let out_task = tokio::spawn(read_capped(child.stdout.take().expect("piped"), cap));
    let err_task = tokio::spawn(read_capped(child.stderr.take().expect("piped"), cap));

    enum End {
        Exited(std::process::ExitStatus),
        TimedOut,
        Cancelled,
    }
    let end = tokio::select! {
        s = child.wait() => End::Exited(s?),
        _ = tokio::time::sleep(timeout) => End::TimedOut,
        _ = cancel.cancelled() => End::Cancelled,
    };
    if !matches!(end, End::Exited(_)) {
        kill_tree(pid);
        let _ = child.kill().await;
    }
    // Background processes may keep the pipes open: don't wait for them forever.
    let grace = Duration::from_secs(2);
    let stdout = tokio::time::timeout(grace, out_task).await.ok().and_then(Result::ok).unwrap_or_default();
    let stderr = tokio::time::timeout(grace, err_task).await.ok().and_then(Result::ok).unwrap_or_default();
    let elapsed = started.elapsed();

    let stdout = String::from_utf8_lossy(&stdout);
    let stderr = String::from_utf8_lossy(&stderr);
    let mut text = String::new();
    if !stdout.trim().is_empty() {
        text.push_str(stdout.trim_end());
        text.push('\n');
    }
    if !stderr.trim().is_empty() {
        text.push_str("[stderr]\n");
        text.push_str(stderr.trim_end());
        text.push('\n');
    }
    let (status_line, code, is_error) = match end {
        End::Exited(s) => {
            let code = s.code();
            (
                format!("[exit code {}]", code.map(|c| c.to_string()).unwrap_or_else(|| "signal".into())),
                code,
                !s.success(),
            )
        }
        End::TimedOut => (format!("[timed out after {}s — process killed]", timeout.as_secs()), None, true),
        End::Cancelled => ("[cancelled]".to_string(), None, true),
    };
    if text.is_empty() {
        text.push_str("(no output)\n");
    }
    text.push_str(&status_line);
    let mut out = ToolOutput::text(truncate_output(&text, max_output)).with_data(json!({
        "command": command,
        "cwd": cwd.display().to_string(),
        "exit_code": code,
        "duration_ms": elapsed.as_millis() as u64,
    }));
    out.is_error = is_error;
    Ok(out)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_and_captures() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let out = run_command(
            "echo hello; echo oops >&2; exit 3",
            dir.path(),
            Duration::from_secs(10),
            10_000,
            None,
            &cancel,
        )
        .await
        .unwrap();
        assert!(out.text.contains("hello"));
        assert!(out.text.contains("[stderr]\noops"));
        assert!(out.text.contains("[exit code 3]"));
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn times_out_and_kills_children() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let started = Instant::now();
        let out = run_command("sleep 30 & sleep 30", dir.path(), Duration::from_millis(300), 10_000, None, &cancel)
            .await
            .unwrap();
        assert!(out.text.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn respects_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();
        let cancel = CancellationToken::new();
        let out = run_command("ls", dir.path(), Duration::from_secs(10), 10_000, None, &cancel).await.unwrap();
        assert!(out.text.contains("marker.txt"));
        assert!(!out.is_error);
    }
}
