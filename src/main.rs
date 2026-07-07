use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ember::agent::Agent;
use ember::app::{App, AppEvent};
use ember::ollama::OllamaClient;
use ember::ui;
use futures_util::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::stdout;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

fn parse_args() -> (String, String) {
    let mut endpoint = "http://localhost:11434".to_string();
    let mut model = "qwen3:14b".to_string();
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--endpoint" if i + 1 < args.len() => {
                endpoint = args[i + 1].clone();
                i += 2;
            }
            "--model" if i + 1 < args.len() => {
                model = args[i + 1].clone();
                i += 2;
            }
            _ => i += 1,
        }
    }
    (endpoint, model)
}

#[tokio::main]
async fn main() -> Result<()> {
    let (endpoint, model) = parse_args();
    let root = std::env::current_dir()?;

    let client = Arc::new(OllamaClient::new(endpoint, model.clone()));
    let agent = Arc::new(Mutex::new(Agent::new(root, 40, true)));
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();

    // Ping in the background so startup isn't blocked on Ollama being slow to respond.
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

    let mut app = App::new(model);
    let mut events = EventStream::new();

    let result = run(&mut terminal, &mut app, &mut events, &mut rx, agent, client, tx).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    events: &mut EventStream,
    rx: &mut mpsc::UnboundedReceiver<AppEvent>,
    agent: Arc<Mutex<Agent>>,
    client: Arc<OllamaClient>,
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
                    match key.code {
                        KeyCode::Esc => app.should_quit = true,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.should_quit = true;
                        }
                        KeyCode::Enter => {
                            if let Some(text) = app.submit() {
                                let agent = agent.clone();
                                let client = client.clone();
                                let tx = tx.clone();
                                tokio::spawn(async move {
                                    let mut agent = agent.lock().await;
                                    let tx2 = tx.clone();
                                    let res = agent
                                        .send(&text, &client, move |ev| { let _ = tx2.send(AppEvent::Agent(ev)); }, |_| true)
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
