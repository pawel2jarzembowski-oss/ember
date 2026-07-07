//! ratatui rendering: status bar, scrollable chat log, input box.

use crate::app::{App, Msg};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(3)])
        .split(f.area());

    render_status(f, app, chunks[0]);
    render_log(f, app, chunks[1]);
    render_input(f, app, chunks[2]);
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let dot = if app.connected { Span::styled("●", Style::default().fg(Color::Green)) } else { Span::styled("●", Style::default().fg(Color::Red)) };
    let busy = if app.busy { " · working…" } else { "" };
    let tokens = app.prompt_tokens + app.completion_tokens;
    let line = Line::from(vec![
        Span::raw(" "),
        dot,
        Span::raw(format!(" {} — {}{}", app.model, app.status_detail, busy)),
        Span::styled(format!("   {tokens} tok"), Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_log(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    if app.messages.is_empty() {
        lines.push(Line::from(Span::styled(
            "Type a task and press Enter. Ember reads/writes files and runs commands in this folder.",
            Style::default().fg(Color::DarkGray),
        )));
    }
    for msg in &app.messages {
        match msg {
            Msg::User(text) => {
                lines.push(Line::from(Span::styled("you", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
                for l in text.lines() {
                    lines.push(Line::from(l.to_string()));
                }
            }
            Msg::Assistant(text) => {
                lines.push(Line::from(Span::styled("ember", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))));
                for l in text.lines() {
                    lines.push(Line::from(l.to_string()));
                }
            }
            Msg::Tool { name, args, result } => {
                let state = if result.is_some() { "✓" } else { "…" };
                lines.push(Line::from(Span::styled(
                    format!("  {state} {name}({args})"),
                    Style::default().fg(Color::Magenta),
                )));
                if let Some(r) = result {
                    for l in r.lines().take(8) {
                        lines.push(Line::from(Span::styled(format!("    {l}"), Style::default().fg(Color::DarkGray))));
                    }
                }
            }
        }
        lines.push(Line::from(""));
    }

    let block = Block::default().borders(Borders::ALL).title(" Ember ");
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    f.render_widget(para, area);
}

fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.busy { " thinking… " } else { " message (Enter to send) " };
    let block = Block::default().borders(Borders::ALL).title(title);
    let para = Paragraph::new(app.input.as_str()).block(block);
    f.render_widget(para, area);
}
