//! Any server speaking the OpenAI `/audio/transcriptions` API
//! (faster-whisper-server, whisper.cpp server, LocalAI, vLLM, ...).

use async_trait::async_trait;

use super::openai_style::{self, Fields};
use super::{Provider, Settings};
use crate::error::SttError;

pub struct OpenAiCompatible(Settings);

impl OpenAiCompatible {
    pub fn new(s: Settings) -> Self {
        Self(s)
    }

    pub(crate) fn fields(&self) -> Fields {
        let mut f: Fields = vec![
            ("model", self.0.model.clone()),
            ("response_format", "json".into()),
        ];
        if let Some(l) = &self.0.language {
            f.push(("language", l.clone()));
        }
        f
    }
}

#[async_trait]
impl Provider for OpenAiCompatible {
    fn name(&self) -> &'static str {
        "openai-compatible"
    }
    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, SttError> {
        openai_style::send("openai-compatible", &self.0, &self.fields(), wav).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Minimal one-shot HTTP server: returns the raw request and replies with `response_body`.
    async fn mock_server(
        status: &str,
        response_body: &str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let status = status.to_string();
        let body = response_body.to_string();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            // Read until headers + full body (Content-Length) arrived.
            loop {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(pos) = text.find("\r\n\r\n") {
                    let len = text[..pos]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if buf.len() >= pos + 4 + len {
                        break;
                    }
                }
            }
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf).to_string()
        });
        (format!("http://{addr}/v1"), handle)
    }

    fn settings(base_url: String) -> Settings {
        Settings {
            api_key: "secret".into(),
            model: "whisper-1".into(),
            base_url,
            language: Some("en".into()),
        }
    }

    #[tokio::test]
    async fn transcribes_against_mock_server() {
        let (url, handle) = mock_server("200 OK", r#"{"text":" hello world "}"#).await;
        let p = OpenAiCompatible::new(settings(url));
        let text = p.transcribe(vec![1, 2, 3, 4]).await.unwrap();
        assert_eq!(text, "hello world");
        let req = handle.await.unwrap();
        assert!(req.starts_with("POST /v1/audio/transcriptions"));
        assert!(req
            .to_ascii_lowercase()
            .contains("authorization: bearer secret"));
        assert!(req.contains("name=\"model\""));
        assert!(req.contains("whisper-1"));
        assert!(req.contains("name=\"file\"; filename=\"audio.wav\""));
    }

    #[tokio::test]
    async fn api_error_is_reported_without_key() {
        let (url, _h) = mock_server(
            "401 Unauthorized",
            r#"{"error":{"message":"Invalid API key"}}"#,
        )
        .await;
        let err = OpenAiCompatible::new(settings(url))
            .transcribe(vec![0; 4])
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("401") && msg.contains("Invalid API key"),
            "{msg}"
        );
        assert!(!msg.contains("secret"));
    }
}
