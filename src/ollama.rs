//! HTTP client for a local Ollama server: model listing and streaming chat with tool-calling.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into(), tool_calls: None, tool_call_id: None }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into(), tool_calls: None, tool_call_id: None }
    }
    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self { role: "tool".into(), content: content.into(), tool_calls: None, tool_call_id: Some(tool_call_id.into()) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: Option<String>,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolDefFunction,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Default, Clone)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[derive(Debug, Default)]
pub struct ChatResult {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    #[serde(default)]
    pub size: u64,
}

pub struct OllamaClient {
    endpoint: String,
    model: String,
    http: reqwest::Client,
}

impl OllamaClient {
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self { endpoint: endpoint.into(), model: model.into(), http: reqwest::Client::new() }
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/api/tags", self.endpoint);
        let resp: Value = self.http.get(&url).send().await?.error_for_status()?.json().await?;
        let models = resp
            .get("models")
            .and_then(|m| m.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(models
            .into_iter()
            .filter_map(|m| serde_json::from_value(m).ok())
            .collect())
    }

    pub async fn ping(&self) -> Result<()> {
        let url = format!("{}/api/show", self.endpoint);
        self.http
            .post(&url)
            .json(&serde_json::json!({ "model": self.model }))
            .send()
            .await?
            .error_for_status()
            .context("Ollama did not respond as expected")?;
        Ok(())
    }

    /// Streams a chat completion, invoking `on_delta` for each piece of assistant text as it
    /// arrives. Returns the full accumulated result once the stream ends.
    pub async fn chat_stream<F: FnMut(&str)>(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        mut on_delta: F,
    ) -> Result<ChatResult> {
        let url = format!("{}/api/chat", self.endpoint);
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": if tools.is_empty() { Value::Null } else { serde_json::to_value(tools)? },
            "stream": true,
            "options": { "temperature": 0.4 },
        });

        let resp = self.http.post(&url).json(&body).send().await?.error_for_status()?;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut result = ChatResult::default();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(idx) = buf.find('\n') {
                let line = buf[..idx].trim().to_string();
                buf = buf[idx + 1..].to_string();
                if line.is_empty() {
                    continue;
                }
                let obj: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue, // incomplete/corrupted line — skip
                };
                if let Some(msg) = obj.get("message") {
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        if !content.is_empty() {
                            result.content.push_str(content);
                            on_delta(content);
                        }
                    }
                    if let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
                        if !calls.is_empty() {
                            result.tool_calls = calls
                                .iter()
                                .cloned()
                                .filter_map(|c| serde_json::from_value(c).ok())
                                .collect();
                        }
                    }
                }
                if obj.get("done").and_then(|d| d.as_bool()).unwrap_or(false) {
                    let prompt = obj.get("prompt_eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let completion = obj.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    result.usage = Some(Usage { prompt_tokens: prompt, completion_tokens: completion });
                }
            }
        }
        Ok(result)
    }
}
