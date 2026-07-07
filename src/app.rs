//! TUI application state and how it reacts to agent/terminal events.

use crate::agent::AgentEvent;

pub enum Msg {
    User(String),
    Assistant(String),
    Tool { name: String, args: String, result: Option<String> },
}

pub enum AppEvent {
    Agent(AgentEvent),
    TurnFinished(Option<String>),
    Connected(bool),
}

pub struct App {
    pub model: String,
    pub connected: bool,
    pub status_detail: String,
    pub messages: Vec<Msg>,
    pub input: String,
    pub busy: bool,
    pub scroll: u16,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    current_assistant: Option<usize>,
    open_tool: Option<usize>,
    pub should_quit: bool,
}

impl App {
    pub fn new(model: String) -> Self {
        Self {
            model,
            connected: false,
            status_detail: "connecting…".into(),
            messages: Vec::new(),
            input: String::new(),
            busy: false,
            scroll: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            current_assistant: None,
            open_tool: None,
            should_quit: false,
        }
    }

    pub fn submit(&mut self) -> Option<String> {
        if self.busy || self.input.trim().is_empty() {
            return None;
        }
        let text = std::mem::take(&mut self.input);
        self.messages.push(Msg::User(text.clone()));
        self.busy = true;
        Some(text)
    }

    pub fn handle_app_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Agent(agent_ev) => self.handle_agent_event(agent_ev),
            AppEvent::TurnFinished(err) => {
                self.busy = false;
                self.current_assistant = None;
                self.open_tool = None;
                if let Some(e) = err {
                    self.messages.push(Msg::Assistant(format!("❌ Error: {e}")));
                }
            }
            AppEvent::Connected(ok) => {
                self.connected = ok;
                self.status_detail = if ok { "connected".into() } else { "unreachable".into() };
            }
        }
    }

    fn handle_agent_event(&mut self, ev: AgentEvent) {
        match ev {
            AgentEvent::AssistantDelta(delta) => {
                if self.current_assistant.is_none() {
                    self.messages.push(Msg::Assistant(String::new()));
                    self.current_assistant = Some(self.messages.len() - 1);
                }
                if let Some(idx) = self.current_assistant {
                    if let Msg::Assistant(text) = &mut self.messages[idx] {
                        text.push_str(&delta);
                    }
                }
            }
            AgentEvent::AssistantDone => {
                self.current_assistant = None;
            }
            AgentEvent::ToolStart { name, args } => {
                self.messages.push(Msg::Tool { name, args, result: None });
                self.open_tool = Some(self.messages.len() - 1);
            }
            AgentEvent::ToolResult { result, .. } => {
                if let Some(idx) = self.open_tool.take() {
                    if let Msg::Tool { result: r, .. } = &mut self.messages[idx] {
                        *r = Some(result);
                    }
                }
            }
            AgentEvent::Usage(u) => {
                self.prompt_tokens += u.prompt_tokens;
                self.completion_tokens += u.completion_tokens;
            }
        }
    }
}
