//! Agent tools, sandboxed to the current working directory (protection against `../..`).

use crate::ollama::{ToolDef, ToolDefFunction};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Runs a tool call. `confirm` is invoked before anything that mutates state or runs a command;
/// returning `false` rejects the action without doing it.
pub fn run_tool(name: &str, args: &Value, root: &Path, confirm: impl FnOnce(&str) -> bool) -> String {
    match name {
        "read_file" => read_file(args, root),
        "write_file" => write_file(args, root, confirm),
        "list_files" => list_files(args, root),
        "run_command" => run_command(args, root, confirm),
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

fn write_file(args: &Value, root: &Path, confirm: impl FnOnce(&str) -> bool) -> String {
    let (Some(path), Some(content)) = (arg_str(args, "path"), arg_str(args, "content")) else {
        return "ERROR: missing \"path\" or \"content\"".into();
    };
    let abs = match resolve_inside(root, path) {
        Ok(p) => p,
        Err(e) => return format!("ERROR: {e}"),
    };
    if !confirm(&format!("Write file: {path}? ({} chars)", content.len())) {
        return "REJECTED: the user did not approve this write.".into();
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

fn run_command(args: &Value, root: &Path, confirm: impl FnOnce(&str) -> bool) -> String {
    let Some(cmd) = arg_str(args, "command") else { return "ERROR: missing \"command\"".into() };
    if !confirm(&format!("Run command: {cmd}")) {
        return "REJECTED: the user did not approve running this command.".into();
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

    fn tmp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ember-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
}
