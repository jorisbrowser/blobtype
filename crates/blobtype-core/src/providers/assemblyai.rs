//! AssemblyAI: upload raw bytes, create a transcript, poll until done.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use super::{check_status, http_client, Provider, Settings};
use crate::error::SttError;

const POLL_INTERVAL: Duration = Duration::from_millis(1000);
const POLL_TIMEOUT: Duration = Duration::from_secs(120);

pub struct AssemblyAi(Settings);

impl AssemblyAi {
    pub fn new(s: Settings) -> Self {
        Self(s)
    }

    /// JSON body of `POST /transcript`. `model` (optional) selects `speech_models`.
    pub(crate) fn create_body(&self, upload_url: &str) -> Value {
        let mut body = json!({ "audio_url": upload_url });
        match &self.0.language {
            Some(l) => body["language_code"] = json!(l),
            None => body["language_detection"] = json!(true),
        }
        if !self.0.model.is_empty() {
            body["speech_models"] = json!([self.0.model]);
        }
        body
    }
}

pub(crate) fn upload_url(body: &Value) -> Result<String, SttError> {
    body.get("upload_url")
        .and_then(|u| u.as_str())
        .map(str::to_string)
        .ok_or(SttError::BadResponse("assemblyai"))
}

pub(crate) enum Poll {
    Pending,
    Done(String),
}

pub(crate) fn interpret_poll(body: &Value) -> Result<Poll, SttError> {
    match body.get("status").and_then(|s| s.as_str()) {
        Some("completed") => Ok(Poll::Done(
            body.get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .trim()
                .to_string(),
        )),
        Some("error") => Err(SttError::Api {
            provider: "assemblyai",
            status: 200,
            message: body
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("transcription failed")
                .to_string(),
        }),
        Some(_) => Ok(Poll::Pending), // queued / processing
        None => Err(SttError::BadResponse("assemblyai")),
    }
}

#[async_trait]
impl Provider for AssemblyAi {
    fn name(&self) -> &'static str {
        "assemblyai"
    }

    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, SttError> {
        let client = http_client();
        let base = &self.0.base_url;
        // The key goes in a bare `authorization` header (no "Bearer").
        let resp = client
            .post(format!("{base}/upload"))
            .header("authorization", &self.0.api_key)
            .header("content-type", "application/octet-stream")
            .body(wav)
            .send()
            .await?;
        let up: Value = check_status("assemblyai", resp).await?.json().await?;
        let url = upload_url(&up)?;

        let resp = client
            .post(format!("{base}/transcript"))
            .header("authorization", &self.0.api_key)
            .json(&self.create_body(&url))
            .send()
            .await?;
        let created: Value = check_status("assemblyai", resp).await?.json().await?;
        let id = created
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or(SttError::BadResponse("assemblyai"))?
            .to_string();

        let started = Instant::now();
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            let resp = client
                .get(format!("{base}/transcript/{id}"))
                .header("authorization", &self.0.api_key)
                .send()
                .await?;
            let body: Value = check_status("assemblyai", resp).await?.json().await?;
            if let Poll::Done(text) = interpret_poll(&body)? {
                return Ok(text);
            }
            if started.elapsed() > POLL_TIMEOUT {
                return Err(SttError::Timeout("assemblyai"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aai(lang: Option<&str>, model: &str) -> AssemblyAi {
        AssemblyAi::new(Settings {
            api_key: "ak".into(),
            model: model.into(),
            base_url: "https://api.assemblyai.com/v2".into(),
            language: lang.map(str::to_string),
        })
    }

    #[test]
    fn body_variants() {
        let b = aai(None, "").create_body("https://u/1");
        assert_eq!(b["audio_url"], "https://u/1");
        assert_eq!(b["language_detection"], true);
        assert!(b.get("speech_models").is_none());
        let b = aai(Some("en"), "universal-3-pro").create_body("u");
        assert_eq!(b["language_code"], "en");
        assert_eq!(b["speech_models"][0], "universal-3-pro");
    }

    #[test]
    fn poll_states() {
        assert!(matches!(
            interpret_poll(&json!({"status":"processing"})).unwrap(),
            Poll::Pending
        ));
        match interpret_poll(&json!({"status":"completed","text":" ok "})).unwrap() {
            Poll::Done(t) => assert_eq!(t, "ok"),
            _ => panic!(),
        }
        assert!(interpret_poll(&json!({"status":"error","error":"bad audio"})).is_err());
        assert!(interpret_poll(&json!({})).is_err());
        assert_eq!(upload_url(&json!({"upload_url":"x"})).unwrap(), "x");
    }
}
