//! ratatui rendering: status bar, scrollable chat log, input box (or confirm/model-picker).

use crate::app::{App, ModelPicker, Msg};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

const MIN_INPUT_HEIGHT: u16 = 3;
const MAX_INPUT_HEIGHT: u16 = 10;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let input_height = if app.model_picker.is_some() {
        MIN_INPUT_HEIGHT
    } else {
        compute_input_height(&app.input, area.width)
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(input_height)])
        .split(area);

    render_status(f, app, chunks[0]);

    if let Some(picker) = &app.model_picker {
        render_model_picker(f, picker, chunks[1]);
        render_picker_hint(f, chunks[2]);
    } else {
        render_log(f, app, chunks[1]);
        if let Some((message, _)) = &app.pending_confirm {
            render_confirm(f, message, chunks[2]);
        } else {
            render_input(f, app, chunks[2]);
        }
    }
}

/// How many text-area rows the input box needs to show everything typed so far, wrapped at the
/// given terminal width, clamped to a sane range so it can't eat the whole screen.
fn compute_input_height(text: &str, width: u16) -> u16 {
    let inner_width = width.saturating_sub(2).max(1);
    let mut wrapped_lines: u16 = 0;
    for line in text.split('\n') {
        let len = line.chars().count() as u16;
        wrapped_lines += if len == 0 { 1 } else { len.div_ceil(inner_width).max(1) };
    }
    (wrapped_lines.max(1) + 2).clamp(MIN_INPUT_HEIGHT, MAX_INPUT_HEIGHT)
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let dot = if app.connected { Span::styled("●", Style::default().fg(Color::Green)) } else { Span::styled("●", Style::default().fg(Color::Red)) };
    let busy = if app.busy { " · working…" } else { "" };
    let tokens = app.prompt_tokens + app.completion_tokens;
    let line = Line::from(vec![
        Span::raw(" "),
        dot,
        Span::raw(format!(" {} — {}{}", app.model, app.status_detail, busy)),
        Span::styled(format!("   write:{}", app.write_perm.label()), Style::default().fg(Color::DarkGray)),
        Span::styled(format!(" cmd:{}", app.command_perm.label()), Style::default().fg(Color::DarkGray)),
        Span::styled(format!("   {tokens} tok"), Style::default().fg(Color::DarkGray)),
        Span::styled("   [F2] write  [F3] cmd  [F4] model", Style::default().fg(Color::DarkGray)),
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
    let title = if app.busy { " thinking… " } else { " message (Enter to send, F4 to switch model) " };
    let block = Block::default().borders(Borders::ALL).title(title);
    let para = Paragraph::new(app.input.as_str()).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn render_confirm(f: &mut Frame, message: &str, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" approve? [y] yes   [n] no ")
        .border_style(Style::default().fg(Color::Yellow));
    let para = Paragraph::new(message).style(Style::default().fg(Color::Yellow)).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn render_model_picker(f: &mut Frame, picker: &ModelPicker, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    if picker.loading {
        lines.push(Line::from(Span::styled("Loading models…", Style::default().fg(Color::DarkGray))));
    } else if let Some(err) = &picker.error {
        lines.push(Line::from(Span::styled(format!("Could not reach Ollama: {err}"), Style::default().fg(Color::Red))));
    } else if picker.models.is_empty() {
        lines.push(Line::from(Span::styled("No models installed. Run `ollama pull <name>` first.", Style::default().fg(Color::DarkGray))));
    } else {
        for (i, m) in picker.models.iter().enumerate() {
            let marker = if i == picker.selected { "› " } else { "  " };
            let size = format_bytes(m.size);
            let style = if i == picker.selected {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(format!("{marker}{:<30} {size}", m.name), style)));
        }
    }

    let block = Block::default().borders(Borders::ALL).title(" Select a model — ↑/↓, Enter, Esc to cancel ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_picker_hint(f: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" model picker ");
    let para = Paragraph::new("↑/↓ move   Enter select   Esc cancel").style(Style::default().fg(Color::DarkGray)).block(block);
    f.render_widget(para, area);
}

fn format_bytes(n: u64) -> String {
    if n == 0 {
        return String::new();
    }
    let gb = n as f64 / 1_073_741_824.0;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{} MB", n / 1_048_576) }
}
