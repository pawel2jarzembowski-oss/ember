//! Agent tools, sandboxed to the current working directory (protection against `../..`), and
//! gated by per-category permission levels (auto / ask / deny) — see [`PermLevel`].

use crate::ollama::{ToolDef, ToolDefFunction};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How much the agent is allowed to do without a human in the loop, per category.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PermLevel {
    /// Just do it — no prompt, the agent reports what it did afterward.
    Auto,
    /// Ask first — the UI shows a prompt and waits for y/n before proceeding.
    Ask,
    /// Never allowed — refused immediately, no prompt at all.
    Deny,
}

impl PermLevel {
    pub fn cycle(self) -> Self {
        match self {
            PermLevel::Auto => PermLevel::Ask,
            PermLevel::Ask => PermLevel::Deny,
            PermLevel::Deny => PermLevel::Auto,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PermLevel::Auto => "auto",
            PermLevel::Ask => "ask",
            PermLevel::Deny => "deny",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(PermLevel::Auto),
            "ask" => Some(PermLevel::Ask),
            "deny" => Some(PermLevel::Deny),
            _ => None,
        }
    }
}

/// Current permission level for each mutating tool category.
#[derive(Clone, Copy, Debug)]
pub struct Permissions {
    pub write: PermLevel,
    pub command: PermLevel,
}

impl Default for Permissions {
    fn default() -> Self {
        Self { write: PermLevel::Auto, command: PermLevel::Auto }
    }
}

/// Asks the human a yes/no question and waits for the answer. Implemented once for real use
/// (backed by the TUI, see `main.rs`) and once for tests (a fixed canned answer).
pub trait Confirm {
    fn ask(&self, message: String) -> impl Future<Output = bool> + Send;
}

/// A permission set to `Deny` never even calls `Confirm::ask` — this exists so tests can prove
/// that (a confirmer that panics if asked would fail the test if deny leaked through).
async fn gate<C: Confirm>(level: PermLevel, message: impl FnOnce() -> String, confirm: &C) -> Result<(), String> {
    match level {
        PermLevel::Deny => Err("REJECTED: disabled by settings (level = deny).".to_string()),
        PermLevel::Auto => Ok(()),
        PermLevel::Ask => {
            if confirm.ask(message()).await {
                Ok(())
            } else {
                Err("REJECTED: the user did not approve this.".to_string())
            }
        }
    }
}

/// Resolves `rel` against `root`, refusing to escape it.
fn resolve_inside(root: &Path, rel: &str) -> Result<PathBuf> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let joined = if rel.is_empty() || rel == "." { root.clone() } else { root.join(rel) };
    let normalized = normalize(&joined);
    if normalized != root && !normalized.starts_with(&root) {
        return Err(anyhow!("path \"{rel}\" escapes the project folder — refused"));
    }
    Ok(normalized)
}

/// Lexically normalizes a path (resolves `.`/`..` components) without requiring the path to
/// exist on disk, so a not-yet-created file can still be sandbox-checked.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            kind: "function".into(),
            function: ToolDefFunction {
                name: "read_file".into(),
                description: "Reads a text file. Returns the content with line numbers.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "File path relative to the project folder" } },
                    "required": ["path"],
                }),
            },
        },
        ToolDef {
            kind: "function".into(),
            function: ToolDefFunction {
                name: "write_file".into(),
                description: "Creates a new file or overwrites an existing one. Always provide the full content.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path relative to the project folder" },
                        "content": { "type": "string", "description": "Full new content of the file" },
                    },
                    "required": ["path", "content"],
                }),
            },
        },
        ToolDef {
            kind: "function".into(),
            function: ToolDefFunction {
                name: "list_files".into(),
                description: "Lists files and folders in a directory (defaults to the current folder).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "Folder path relative to the project, defaults to \".\"" } },
                }),
            },
        },
        ToolDef {
            kind: "function".into(),
            function: ToolDefFunction {
                name: "run_command".into(),
                description: "Runs a shell command in the project folder and returns its output (stdout+stderr).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "command": { "type": "string", "description": "Command to run, e.g. \"cargo test\"" } },
                    "required": ["command"],
                }),
            },
        },
    ]
}

/// Runs a tool call, gating write_file/run_command behind `perms` and `confirm`.
pub async fn run_tool<C: Confirm>(name: &str, args: &Value, root: &Path, perms: Permissions, confirm: &C) -> String {
    match name {
        "read_file" => read_file(args, root),
        "write_file" => write_file(args, root, perms, confirm).await,
        "list_files" => list_files(args, root),
        "run_command" => run_command(args, root, perms, confirm).await,
        other => format!("ERROR: unknown tool \"{other}\""),
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn read_file(args: &Value, root: &Path) -> String {
    let Some(path) = arg_str(args, "path") else { return "ERROR: missing \"path\"".into() };
    let abs = match resolve_inside(root, path) {
        Ok(p) => p,
        Err(e) => return format!("ERROR: {e}"),
    };
    match std::fs::read_to_string(&abs) {
        Ok(text) => text
            .lines()
            .enumerate()
            .map(|(i, l)| format!("{:>4} | {}", i + 1, l))
            .collect::<Vec<_>>()
            .join("\n"),
        Err(e) => format!("ERROR reading {path}: {e}"),
    }
}

async fn write_file<C: Confirm>(args: &Value, root: &Path, perms: Permissions, confirm: &C) -> String {
    let (Some(path), Some(content)) = (arg_str(args, "path"), arg_str(args, "content")) else {
        return "ERROR: missing \"path\" or \"content\"".into();
    };
    let abs = match resolve_inside(root, path) {
        Ok(p) => p,
        Err(e) => return format!("ERROR: {e}"),
    };
    if let Err(rejected) = gate(perms.write, || format!("Write file: {path}? ({} chars)", content.len()), confirm).await {
        return rejected;
    }
    if let Some(parent) = abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return format!("ERROR creating parent directory for {path}: {e}");
        }
    }
    match std::fs::write(&abs, content) {
        Ok(()) => format!("OK: wrote {path} ({} chars).", content.len()),
        Err(e) => format!("ERROR writing {path}: {e}"),
    }
}

fn list_files(args: &Value, root: &Path) -> String {
    let rel = arg_str(args, "path").unwrap_or(".");
    let abs = match resolve_inside(root, rel) {
        Ok(p) => p,
        Err(e) => return format!("ERROR: {e}"),
    };
    let entries = match std::fs::read_dir(&abs) {
        Ok(e) => e,
        Err(e) => return format!("ERROR listing {rel}: {e}"),
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() { format!("{name}/") } else { name }
        })
        .collect();
    names.sort();
    if names.is_empty() { "(empty)".into() } else { names.join("\n") }
}

async fn run_command<C: Confirm>(args: &Value, root: &Path, perms: Permissions, confirm: &C) -> String {
    let Some(cmd) = arg_str(args, "command") else { return "ERROR: missing \"command\"".into() };
    if let Err(rejected) = gate(perms.command, || format!("Run command: {cmd}"), confirm).await {
        return rejected;
    }
    let (shell, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
    let output = Command::new(shell).arg(flag).arg(cmd).current_dir(root).output();
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let mut res = String::new();
            if !stdout.trim().is_empty() {
                res.push_str(&format!("STDOUT:\n{}\n", stdout.trim()));
            }
            if !stderr.trim().is_empty() {
                res.push_str(&format!("STDERR:\n{}\n", stderr.trim()));
            }
            if !out.status.success() {
                res.push_str(&format!("EXIT CODE: {}\n", out.status.code().unwrap_or(-1)));
            }
            if res.is_empty() { "(no output, command finished)".into() } else { res }
        }
        Err(e) => format!("ERROR running command: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn tmp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ember-test-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() as u64
    }

    #[test]
    fn resolves_a_normal_relative_path_under_root() {
        let root = tmp_root();
        let abs = resolve_inside(&root, "src/main.rs").unwrap();
        assert!(abs.starts_with(root.canonicalize().unwrap()));
        assert!(abs.ends_with("src/main.rs") || abs.ends_with("src\\main.rs"));
    }

    #[test]
    fn dot_resolves_to_root_itself() {
        let root = tmp_root();
        let abs = resolve_inside(&root, ".").unwrap();
        assert_eq!(abs, root.canonicalize().unwrap());
    }

    #[test]
    fn rejects_a_simple_parent_escape() {
        let root = tmp_root();
        assert!(resolve_inside(&root, "../outside.txt").is_err());
    }

    #[test]
    fn rejects_a_deep_parent_escape() {
        let root = tmp_root();
        assert!(resolve_inside(&root, "../../../etc/passwd").is_err());
    }

    #[test]
    fn rejects_a_sibling_folder_sharing_a_name_prefix() {
        let root = tmp_root();
        let sibling = format!("../{}-evil/file.txt", root.file_name().unwrap().to_string_lossy());
        assert!(resolve_inside(&root, &sibling).is_err());
    }

    #[test]
    fn allows_a_nested_subfolder_path() {
        let root = tmp_root();
        let abs = resolve_inside(&root, "a/b/c.txt").unwrap();
        assert!(abs.starts_with(root.canonicalize().unwrap()));
    }

    /// A confirmer that panics if it's ever asked — proves `deny`/`auto` never prompt.
    struct PanicIfAsked;
    impl Confirm for PanicIfAsked {
        async fn ask(&self, message: String) -> bool {
            panic!("should never have asked, but was asked: {message}");
        }
    }

    struct Fixed(bool);
    impl Confirm for Fixed {
        async fn ask(&self, _message: String) -> bool {
            self.0
        }
    }

    /// Records whether it was asked, then answers with a fixed value.
    struct Recording {
        answer: bool,
        was_asked: AtomicBool,
    }
    impl Confirm for Recording {
        async fn ask(&self, _message: String) -> bool {
            self.was_asked.store(true, Ordering::SeqCst);
            self.answer
        }
    }

    #[tokio::test]
    async fn write_file_denied_never_asks_and_never_writes() {
        let root = tmp_root();
        let perms = Permissions { write: PermLevel::Deny, command: PermLevel::Auto };
        let args = json!({ "path": "denied.txt", "content": "hello" });
        let result = write_file(&args, &root, perms, &PanicIfAsked).await;
        assert!(result.starts_with("REJECTED"));
        assert!(!root.join("denied.txt").exists());
    }

    #[tokio::test]
    async fn write_file_auto_never_asks_and_writes_immediately() {
        let root = tmp_root();
        let perms = Permissions { write: PermLevel::Auto, command: PermLevel::Auto };
        let args = json!({ "path": "auto.txt", "content": "hello" });
        let result = write_file(&args, &root, perms, &PanicIfAsked).await;
        assert!(result.starts_with("OK"));
        assert_eq!(std::fs::read_to_string(root.join("auto.txt")).unwrap(), "hello");
    }

    #[tokio::test]
    async fn write_file_ask_writes_only_if_approved() {
        let root = tmp_root();
        let perms = Permissions { write: PermLevel::Ask, command: PermLevel::Auto };

        let args = json!({ "path": "approved.txt", "content": "yes" });
        let confirmer = Recording { answer: true, was_asked: AtomicBool::new(false) };
        let result = write_file(&args, &root, perms, &confirmer).await;
        assert!(result.starts_with("OK"));
        assert!(confirmer.was_asked.load(Ordering::SeqCst));
        assert!(root.join("approved.txt").exists());

        let args2 = json!({ "path": "rejected.txt", "content": "no" });
        let result2 = write_file(&args2, &root, perms, &Fixed(false)).await;
        assert!(result2.starts_with("REJECTED"));
        assert!(!root.join("rejected.txt").exists());
    }

    #[tokio::test]
    async fn run_command_denied_never_asks_and_never_runs() {
        let root = tmp_root();
        let perms = Permissions { write: PermLevel::Auto, command: PermLevel::Deny };
        let args = json!({ "command": "echo should-not-run" });
        let result = run_command(&args, &root, perms, &PanicIfAsked).await;
        assert!(result.starts_with("REJECTED"));
    }

    #[tokio::test]
    async fn run_command_auto_runs_without_asking() {
        let root = tmp_root();
        let perms = Permissions { write: PermLevel::Auto, command: PermLevel::Auto };
        let cmd = if cfg!(windows) { "echo hello-ember" } else { "echo hello-ember" };
        let args = json!({ "command": cmd });
        let result = run_command(&args, &root, perms, &PanicIfAsked).await;
        assert!(result.contains("hello-ember"), "unexpected output: {result}");
    }
}
