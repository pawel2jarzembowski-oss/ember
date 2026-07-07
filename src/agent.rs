//! The agentic loop: the model thinks -> calls tools -> gets results -> repeats until it
//! replies with plain text, mirroring the same loop used by editor-integrated agents.

use crate::ollama::{ChatMessage, OllamaClient, Usage};
use crate::tools::{run_tool, tool_defs};
use std::path::PathBuf;

const SYSTEM_PROMPT: &str = "You are Ember, an autonomous coding agent running in a terminal, fully locally. \
Every path you use is relative to the current project folder — that folder, and nothing outside it, is the \
entire project. You have these tools: read_file, write_file, list_files, run_command. Always investigate with \
read_file/list_files before assuming what's in a file. Use write_file with the full file content. After making \
changes you can run tests/build via run_command and react to the output. Work independently through each step \
until the task is done, then reply in plain text with a short summary and call no more tools.";

pub enum AgentEvent {
    AssistantDelta(String),
    AssistantDone,
    ToolStart { name: String, args: String },
    ToolResult { name: String, result: String },
    Usage(Usage),
}

pub struct Agent {
    history: Vec<ChatMessage>,
    root: PathBuf,
    max_steps: usize,
    auto_approve: bool,
}

impl Agent {
    pub fn new(root: PathBuf, max_steps: usize, auto_approve: bool) -> Self {
        Self { history: vec![ChatMessage::system(SYSTEM_PROMPT)], root, max_steps, auto_approve }
    }

    pub fn reset(&mut self) {
        self.history = vec![ChatMessage::system(SYSTEM_PROMPT)];
    }

    /// Runs one full turn, calling `on_event` for everything the UI should react to, and
    /// `confirm` before any mutating tool call when auto-approve is off.
    pub async fn send(
        &mut self,
        user_text: &str,
        client: &OllamaClient,
        mut on_event: impl FnMut(AgentEvent),
        mut confirm: impl FnMut(&str) -> bool,
    ) -> anyhow::Result<()> {
        self.history.push(ChatMessage::user(user_text));

        for _ in 0..self.max_steps {
            let tools = tool_defs();
            let mut acc = String::new();
            let res = client
                .chat_stream(&self.history, &tools, |delta| {
                    acc.push_str(delta);
                    on_event(AgentEvent::AssistantDelta(delta.to_string()));
                })
                .await?;

            if let Some(usage) = &res.usage {
                on_event(AgentEvent::Usage(usage.clone()));
            }

            self.history.push(ChatMessage {
                role: "assistant".into(),
                content: res.content.clone(),
                tool_calls: if res.tool_calls.is_empty() { None } else { Some(res.tool_calls.clone()) },
                tool_call_id: None,
            });

            if !res.content.trim().is_empty() {
                on_event(AgentEvent::AssistantDone);
            }

            if res.tool_calls.is_empty() {
                return Ok(());
            }

            for tc in &res.tool_calls {
                let name = tc.function.name.clone();
                let args = tc.function.arguments.clone();
                on_event(AgentEvent::ToolStart { name: name.clone(), args: args.to_string() });
                let auto_approve = self.auto_approve;
                let result = run_tool(&name, &args, &self.root, |msg| auto_approve || confirm(msg));
                on_event(AgentEvent::ToolResult { name: name.clone(), result: result.clone() });
                let call_id = tc.id.clone().unwrap_or_else(|| "call_0".to_string());
                self.history.push(ChatMessage::tool(result, call_id));
            }
        }
        on_event(AgentEvent::AssistantDelta(format!(
            "\n⚠️ Reached the {}-step limit. Say \"continue\" to keep going.",
            self.max_steps
        )));
        on_event(AgentEvent::AssistantDone);
        Ok(())
    }
}
