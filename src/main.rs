use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ember::agent::Agent;
use ember::app::{App, AppEvent};
use ember::ollama::OllamaClient;
use ember::tools::{Confirm, PermLevel, Permissions};
use ember::ui;
use futures_util::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::stdout;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

struct Args {
    endpoint: String,
    model: String,
    write: PermLevel,
    command: PermLevel,
}

fn parse_args() -> Args {
    let mut args = Args {
        endpoint: "http://localhost:11434".to_string(),
        model: "qwen3:14b".to_string(),
        write: PermLevel::Auto,
        command: PermLevel::Auto,
    };
    let raw: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "--endpoint" if i + 1 < raw.len() => {
                args.endpoint = raw[i + 1].clone();
                i += 2;
            }
            "--model" if i + 1 < raw.len() => {
                args.model = raw[i + 1].clone();
                i += 2;
            }
            "--write-mode" if i + 1 < raw.len() => {
                if let Some(level) = PermLevel::parse(&raw[i + 1]) {
                    args.write = level;
                }
                i += 2;
            }
            "--command-mode" if i + 1 < raw.len() => {
                if let Some(level) = PermLevel::parse(&raw[i + 1]) {
                    args.command = level;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    args
}

/// Bridges the tool-confirmation protocol onto the app-event channel: asking blocks the calling
/// (spawned) task until the UI thread sends back the user's y/n answer.
struct ChannelConfirm {
    tx: mpsc::UnboundedSender<AppEvent>,
}

impl Confirm for ChannelConfirm {
    async fn ask(&self, message: String) -> bool {
        let (resp_tx, resp_rx) = oneshot::channel();
        if self.tx.send(AppEvent::ConfirmRequest { message, respond: resp_tx }).is_err() {
            return false;
        }
        resp_rx.await.unwrap_or(false)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();
    let root = std::env::current_dir()?;

    let client = Arc::new(OllamaClient::new(args.endpoint, args.model.clone()));
    let agent = Arc::new(tokio::sync::Mutex::new(Agent::new(root, 40)));
    let perms = Arc::new(Mutex::new(Permissions { write: args.write, command: args.command }));
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();

    {
        let client = client.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let ok = client.ping().await.is_ok();
            let _ = tx.send(AppEvent::Connected(ok));
        });
    }

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(args.model);
    app.write_perm = args.write;
    app.command_perm = args.command;
    let mut events = EventStream::new();

    let result = run(&mut terminal, &mut app, &mut events, &mut rx, agent, client, perms, tx).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    result
}

#[allow(clippy::too_many_arguments)]
async fn run(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    events: &mut EventStream,
    rx: &mut mpsc::UnboundedReceiver<AppEvent>,
    agent: Arc<tokio::sync::Mutex<Agent>>,
    client: Arc<OllamaClient>,
    perms: Arc<Mutex<Permissions>>,
    tx: mpsc::UnboundedSender<AppEvent>,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;
        if app.should_quit {
            return Ok(());
        }

        tokio::select! {
            maybe_event = events.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_event {
                    if key.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }

                    if app.pending_confirm.is_some() {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => app.answer_confirm(true),
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.answer_confirm(false),
                            _ => {}
                        }
                        continue;
                    }

                    if let Some(picker) = &mut app.model_picker {
                        match key.code {
                            KeyCode::Up => {
                                if picker.selected > 0 { picker.selected -= 1; }
                            }
                            KeyCode::Down => {
                                if picker.selected + 1 < picker.models.len() { picker.selected += 1; }
                            }
                            KeyCode::Enter => {
                                if let Some(m) = picker.models.get(picker.selected) {
                                    client.set_model(m.name.clone());
                                    app.model = m.name.clone();
                                }
                                app.model_picker = None;
                            }
                            KeyCode::Esc => app.model_picker = None,
                            _ => {}
                        }
                        continue;
                    }

                    match key.code {
                        KeyCode::Esc => app.should_quit = true,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.should_quit = true;
                        }
                        KeyCode::F(2) => {
                            let mut p = perms.lock().unwrap();
                            p.write = p.write.cycle();
                            app.write_perm = p.write;
                        }
                        KeyCode::F(3) => {
                            let mut p = perms.lock().unwrap();
                            p.command = p.command.cycle();
                            app.command_perm = p.command;
                        }
                        KeyCode::F(4) => {
                            app.open_model_picker();
                            let client = client.clone();
                            let tx = tx.clone();
                            tokio::spawn(async move {
                                let result = client.list_models().await.map_err(|e| e.to_string());
                                let _ = tx.send(AppEvent::ModelsLoaded(result));
                            });
                        }
                        KeyCode::Enter => {
                            if let Some(text) = app.submit() {
                                let agent = agent.clone();
                                let client = client.clone();
                                let perms = perms.clone();
                                let tx = tx.clone();
                                tokio::spawn(async move {
                                    let mut agent = agent.lock().await;
                                    let confirmer = ChannelConfirm { tx: tx.clone() };
                                    let perms_fn = move || *perms.lock().unwrap();
                                    let tx2 = tx.clone();
                                    let res = agent
                                        .send(&text, &client, perms_fn, &confirmer, move |ev| {
                                            let _ = tx2.send(AppEvent::Agent(ev));
                                        })
                                        .await;
                                    let _ = tx.send(AppEvent::TurnFinished(res.err().map(|e| e.to_string())));
                                });
                            }
                        }
                        KeyCode::Backspace => { app.input.pop(); }
                        KeyCode::Char(c) => app.input.push(c),
                        KeyCode::Up => app.scroll = app.scroll.saturating_sub(1),
                        KeyCode::Down => app.scroll = app.scroll.saturating_add(1),
                        _ => {}
                    }
                }
            }
            Some(app_event) = rx.recv() => {
                app.handle_app_event(app_event);
            }
        }
    }
}
