# Ember

A terminal chat client and lightweight coding agent for your local [Ollama](https://ollama.com) models, written in Rust with a real TUI (built on [ratatui](https://ratatui.rs)).

Think of it as a small, terminal-native cousin of a full editor-integrated coding agent: same idea — an agentic loop that reads files, writes files, and runs commands on its own, step by step — but it lives in your terminal instead of an editor window, so it works over SSH, in tmux, anywhere you've got a shell.

## What it does
- **Streamed chat** — the model's reply appears token by token as it's generated.
- **Agent tools** — `read_file`, `write_file`, `list_files`, `run_command`, all sandboxed to the folder you launch Ember from (no `../` escapes — see `src/tools.rs`).
- **Permission modes, chosen live** — write/edit and shell commands are each independently `auto` (just do it), `ask` (approve first), or `deny` (never). Press **F2**/**F3** in the app to cycle them without restarting.
- **Model picker** — **F4** lists every model already pulled into Ollama and lets you switch the active one on the fly, no restart needed.
- **Live status bar** — model name, connection state, current permission modes, and a running token count for the session.
- **Full-screen TUI** — scrollable chat log, an input box that grows as you type (wraps and expands up to a cap instead of staying cramped), all keyboard-driven.

## Install / run
Requires a running Ollama instance (`ollama run qwen3:14b` once, to pull a tool-calling-capable model).

```bash
git clone https://github.com/pawel2jarzembowski-oss/ember.git
cd ember
cargo run --release
```

Optional flags (set the *initial* permission levels; both default to `auto`):
```bash
cargo run --release -- --endpoint http://localhost:11434 --model qwen3:14b --write-mode ask --command-mode ask
```

Ember works on whatever folder you run it from — `cd` into a project first.

### Keys
- Type to compose a message (the box grows as you type), **Enter** to send.
- **F2** — cycle the write/edit permission (auto → ask → deny → auto).
- **F3** — cycle the shell command permission, same cycle.
- **F4** — open the model picker; **↑/↓** to move, **Enter** to switch, **Esc** to cancel.
- When something needs approval: **y** to approve, **n** or **Esc** to reject.
- **↑ / ↓** to scroll the chat log (when not in the model picker).
- **Esc** or **Ctrl+C** to quit (when nothing is pending approval or open).

## Safety
- File operations are restricted to the folder Ember was launched from.
- Both `write_file` and `run_command` are gated by their own permission level (`auto` / `ask` / `deny`), defaulting to `auto`. Set `--write-mode ask --command-mode ask` (or press F2/F3 once running) if you want to review everything before it happens.
- `deny` refuses immediately — the agent never even gets to ask.
- Everything stays local — Ember only talks to the Ollama endpoint you point it at.

## Testing
```bash
cargo test
```
Runs the path-sandboxing and permission-gating unit tests (`src/tools.rs` — proves `deny` never prompts and never mutates, `auto` never prompts but does mutate, `ask` only mutates if approved) and integration tests for the streaming Ollama client against a fake local server (`tests/ollama_test.rs`) — no real Ollama install required to run the suite.

## License
[MIT](LICENSE)
