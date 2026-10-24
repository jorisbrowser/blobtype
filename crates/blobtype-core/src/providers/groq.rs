//! Groq: OpenAI-compatible endpoint, `whisper-large-v3-turbo` by default.

use async_trait::async_trait;

use super::openai_style::{self, Fields};
use super::{Provider, Settings};
use crate::error::SttError;

pub struct Groq(Settings);

impl Groq {
    pub fn new(s: Settings) -> Self {
        Self(s)
    }

    pub(crate) fn fields(&self) -> Fields {
        let mut f: Fields = vec![
            ("model", self.0.model.clone()),
            ("response_format", "json".into()),
            ("temperature", "0".into()),
        ];
        if let Some(l) = &self.0.language {
            f.push(("language", l.clone()));
        }
        f
    }
}

#[async_trait]
impl Provider for Groq {
    fn name(&self) -> &'static str {
        "groq"
    }
    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, SttError> {
        openai_style::send("groq", &self.0, &self.fields(), wav).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(lang: Option<&str>) -> Settings {
        Settings {
            api_key: "k".into(),
            model: "whisper-large-v3-turbo".into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            language: lang.map(str::to_string),
        }
    }

    #[test]
    fn fields_and_request() {
        let g = Groq::new(settings(Some("de")));
        let f = g.fields();
        assert!(f.contains(&("model", "whisper-large-v3-turbo".into())));
        assert!(f.contains(&("language", "de".into())));
        let client = reqwest::Client::new();
        let req = openai_style::build_request(&client, &g.0, &f, vec![0; 4])
            .build()
            .unwrap();
        assert_eq!(
            req.url().as_str(),
            "https://api.groq.com/openai/v1/audio/transcriptions"
        );
        assert_eq!(req.headers()["authorization"], "Bearer k");
        assert!(req.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("multipart/form-data"));
    }

    #[test]
    fn auto_language_omits_field() {
        assert!(!Groq::new(settings(None))
            .fields()
            .iter()
            .any(|(k, _)| *k == "language"));
    }
}
