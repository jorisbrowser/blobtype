//! ElevenLabs Scribe: `POST /v1/speech-to-text`, `xi-api-key` header, multipart.

use async_trait::async_trait;
use reqwest::multipart::{Form, Part};

use super::openai_style::parse_text;
use super::{check_status, http_client, Provider, Settings};
use crate::error::SttError;

pub struct ElevenLabs(Settings);

impl ElevenLabs {
    pub fn new(s: Settings) -> Self {
        Self(s)
    }

    pub(crate) fn request(
        &self,
        client: &reqwest::Client,
        wav: Vec<u8>,
    ) -> reqwest::RequestBuilder {
        let mut form = Form::new().text("model_id", self.0.model.clone());
        if let Some(l) = &self.0.language {
            form = form.text("language_code", l.clone());
        }
        let part = Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .expect("static mime");
        client
            .post(format!("{}/speech-to-text", self.0.base_url))
            .header("xi-api-key", &self.0.api_key)
            .multipart(form.part("file", part))
    }
}

#[async_trait]
impl Provider for ElevenLabs {
    fn name(&self) -> &'static str {
        "elevenlabs"
    }
    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, SttError> {
        let resp = self.request(&http_client(), wav).send().await?;
        let body = check_status("elevenlabs", resp).await?.text().await?;
        parse_text("elevenlabs", &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_shape() {
        let e = ElevenLabs::new(Settings {
            api_key: "xk".into(),
            model: "scribe_v2".into(),
            base_url: "https://api.elevenlabs.io/v1".into(),
            language: None,
        });
        let req = e
            .request(&reqwest::Client::new(), vec![0; 2])
            .build()
            .unwrap();
        assert_eq!(
            req.url().as_str(),
            "https://api.elevenlabs.io/v1/speech-to-text"
        );
        assert_eq!(req.headers()["xi-api-key"], "xk");
        assert!(req.headers().get("authorization").is_none());
    }
}
