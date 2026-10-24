//! OpenAI: `gpt-transcribe` (default), `gpt-4o-transcribe`, `gpt-4o-mini-transcribe`, `whisper-1`.

use async_trait::async_trait;

use super::openai_style::{self, Fields};
use super::{Provider, Settings};
use crate::error::SttError;

pub struct OpenAi(Settings);

impl OpenAi {
    pub fn new(s: Settings) -> Self {
        Self(s)
    }

    pub(crate) fn fields(&self) -> Fields {
        let mut f: Fields = vec![
            ("model", self.0.model.clone()),
            ("response_format", "json".into()),
        ];
        if let Some(l) = &self.0.language {
            // `gpt-transcribe` takes a `languages` list and rejects `language`;
            // every older model takes the singular field.
            if self.0.model.starts_with("gpt-transcribe") {
                f.push(("languages[]", l.clone()));
            } else {
                f.push(("language", l.clone()));
            }
        }
        f
    }
}

#[async_trait]
impl Provider for OpenAi {
    fn name(&self) -> &'static str {
        "openai"
    }
    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, SttError> {
        openai_style::send("openai", &self.0, &self.fields(), wav).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(model: &str) -> Settings {
        Settings {
            api_key: "k".into(),
            model: model.into(),
            base_url: "https://api.openai.com/v1".into(),
            language: Some("en".into()),
        }
    }

    #[test]
    fn language_field_depends_on_model() {
        let f = OpenAi::new(s("gpt-transcribe")).fields();
        assert!(f.contains(&("languages[]", "en".into())));
        assert!(!f.iter().any(|(k, _)| *k == "language"));
        let f = OpenAi::new(s("gpt-4o-transcribe")).fields();
        assert!(f.contains(&("language", "en".into())));
    }
}
