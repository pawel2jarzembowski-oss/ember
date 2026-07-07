# Ember

A terminal chat client and lightweight coding agent for your local [Ollama](https://ollama.com) models, written in Rust with a real TUI (built on [ratatui](https://ratatui.rs)).

Think of it as a small, terminal-native cousin of a full editor-integrated coding agent: same idea — an agentic loop that reads files, writes files, and runs commands on its own, step by step — but it lives in your terminal instead of an editor window, so it works over SSH, in tmux, anywhere you've got a shell.

## What it does
- **Streamed chat** — the model's reply appears token by token as it's generated.
- **Agent tools** — `read_file`, `write_file`, `list_files`, `run_command`, all sandboxed to the folder you launch Ember from (no `../` escapes — see `src/tools.rs`).
- **Live status bar** — shows the model name, connection state, and a running token count for the session.
- **Full-screen TUI** — scrollable chat log, dedicated input box, all keyboard-driven.

## Install / run
Requires a running Ollama instance (`ollama run qwen3:14b` once, to pull a tool-calling-capable model).

```bash
git clone https://github.com/pawel2jarzembowski-oss/ember.git
cd ember
cargo run --release
```

Optional flags:
```bash
cargo run --release -- --endpoint http://localhost:11434 --model qwen3:14b
```

Ember works on whatever folder you run it from — `cd` into a project first.

### Keys
- Type to compose a message, **Enter** to send.
- **↑ / ↓** to scroll the chat log.
- **Esc** or **Ctrl+C** to quit.

## Safety
- File operations are restricted to the folder Ember was launched from.
- v1 runs in **auto-approve mode only** — the agent writes/edits files and runs commands without an interactive confirmation prompt (an "ask first" mode is a natural next step, tracked as future work). Run it in a folder you don't mind it touching, or under version control.
- Everything stays local — Ember only talks to the Ollama endpoint you point it at.

## Testing
```bash
cargo test
```
Runs the path-sandboxing unit tests (`src/tools.rs`) and integration tests for the streaming Ollama client against a fake local server (`tests/ollama_test.rs`) — no real Ollama install required to run the suite.

## License
[MIT](LICENSE)
