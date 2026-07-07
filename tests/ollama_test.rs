//! Integration tests for OllamaClient against a fake local TCP server speaking Ollama's
//! newline-delimited-JSON streaming format, so these run without a real Ollama install.

use ember::ollama::{ChatMessage, OllamaClient};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn start_fake_server(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = vec![0u8; 4096];
            loop {
                match socket.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn chat_stream_accumulates_content_and_parses_a_tool_call() {
    let body = concat!(
        "{\"message\":{\"role\":\"assistant\",\"content\":\"Sure, \"}}\n",
        "{\"message\":{\"role\":\"assistant\",\"content\":\"creating it.\"}}\n",
        "{\"message\":{\"role\":\"assistant\",\"content\":\"\",\"tool_calls\":[{\"function\":{\"name\":\"write_file\",\"arguments\":{\"path\":\"a.txt\",\"content\":\"hi\"}}}]},\"done\":true,\"prompt_eval_count\":10,\"eval_count\":5}\n",
    );
    let endpoint = start_fake_server(body).await;
    let client = OllamaClient::new(endpoint, "test-model");
    let mut streamed = String::new();
    let messages = vec![ChatMessage::user("make a file")];

    let result = client.chat_stream(&messages, &[], |delta| streamed.push_str(delta)).await.unwrap();

    assert_eq!(streamed, "Sure, creating it.");
    assert_eq!(result.content, "Sure, creating it.");
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].function.name, "write_file");
    assert_eq!(result.tool_calls[0].function.arguments["path"], "a.txt");

    let usage = result.usage.expect("usage should be captured from the done:true chunk");
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
}

#[tokio::test]
async fn chat_stream_with_no_tool_calls_returns_empty_vec() {
    let body = "{\"message\":{\"role\":\"assistant\",\"content\":\"hi there\"},\"done\":true,\"prompt_eval_count\":2,\"eval_count\":3}\n";
    let endpoint = start_fake_server(body).await;
    let client = OllamaClient::new(endpoint, "test-model");
    let messages = vec![ChatMessage::user("hi")];

    let result = client.chat_stream(&messages, &[], |_| {}).await.unwrap();

    assert_eq!(result.content, "hi there");
    assert!(result.tool_calls.is_empty());
}

#[tokio::test]
async fn list_models_parses_tags_response() {
    let body = "{\"models\":[{\"name\":\"qwen3:14b\",\"size\":123},{\"name\":\"llama3.1:8b\",\"size\":456}]}";
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = vec![0u8; 4096];
            loop {
                match socket.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    let client = OllamaClient::new(format!("http://{addr}"), "test-model");

    let models = client.list_models().await.unwrap();

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].name, "qwen3:14b");
    assert_eq!(models[1].size, 456);
}
