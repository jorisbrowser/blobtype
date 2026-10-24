//! Mistral Voxtral: `POST /v1/audio/transcriptions` (multipart, Bearer auth).

use async_trait::async_trait;

use super::openai_style::{self, Fields};
use super::{Provider, Settings};
use crate::error::SttError;

pub struct Mistral(Settings);

impl Mistral {
    pub fn new(s: Settings) -> Self {
        Self(s)
    }

    pub(crate) fn fields(&self) -> Fields {
        let mut f: Fields = vec![("model", self.0.model.clone())];
        if let Some(l) = &self.0.language {
            f.push(("language", l.clone()));
        }
        f
    }
}

#[async_trait]
impl Provider for Mistral {
    fn name(&self) -> &'static str {
        "mistral"
    }
    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, SttError> {
        openai_style::send("mistral", &self.0, &self.fields(), wav).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields() {
        let m = Mistral::new(Settings {
            api_key: "k".into(),
            model: "voxtral-mini-latest".into(),
            base_url: "https://api.mistral.ai/v1".into(),
            language: Some("fr".into()),
        });
        assert_eq!(
            m.fields(),
            vec![
                ("model", "voxtral-mini-latest".to_string()),
                ("language", "fr".to_string())
            ]
        );
    }
}
