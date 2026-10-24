//! Google Gemini: audio understanding via `generateContent` with inline audio.
//! The model transcribes when prompted to; the model id is configurable.

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};

use super::{check_status, http_client, Provider, Settings};
use crate::error::SttError;

pub struct Google(Settings);

impl Google {
    pub fn new(s: Settings) -> Self {
        Self(s)
    }

    pub(crate) fn prompt(&self) -> String {
        let lang = match &self.0.language {
            Some(l) => format!(" The language is \"{l}\"."),
            None => String::new(),
        };
        format!(
            "Transcribe this audio verbatim.{lang} Output only the spoken text, \
             with natural punctuation and no commentary, labels or quotation marks. \
             If there is no speech, output nothing."
        )
    }

    pub(crate) fn body(&self, wav: &[u8]) -> Value {
        json!({
            "contents": [{
                "parts": [
                    { "text": self.prompt() },
                    { "inline_data": {
                        "mime_type": "audio/wav",
                        "data": base64::engine::general_purpose::STANDARD.encode(wav),
                    }}
                ]
            }],
            "generationConfig": { "temperature": 0 }
        })
    }

    pub(crate) fn request(&self, client: &reqwest::Client, wav: &[u8]) -> reqwest::RequestBuilder {
        client
            .post(format!(
                "{}/models/{}:generateContent",
                self.0.base_url, self.0.model
            ))
            .header("x-goog-api-key", &self.0.api_key)
            .json(&self.body(wav))
    }
}

pub(crate) fn parse_response(v: &Value) -> Result<String, SttError> {
    let parts = v
        .pointer("/candidates/0/content/parts")
        .and_then(|p| p.as_array());
    match parts {
        Some(parts) => Ok(parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<String>()
            .trim()
            .to_string()),
        // A blocked or silent clip has candidates without content.
        None if v.get("candidates").is_some() => Ok(String::new()),
        None => Err(SttError::BadResponse("google")),
    }
}

#[async_trait]
impl Provider for Google {
    fn name(&self) -> &'static str {
        "google"
    }
    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, SttError> {
        let resp = self.request(&http_client(), &wav).send().await?;
        let v: Value = check_status("google", resp).await?.json().await?;
        parse_response(&v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g() -> Google {
        Google::new(Settings {
            api_key: "gk".into(),
            model: "gemini-3.8-flash".into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            language: Some("de".into()),
        })
    }

    #[test]
    fn request_shape() {
        let req = g()
            .request(&reqwest::Client::new(), &[1, 2, 3])
            .build()
            .unwrap();
        assert_eq!(
            req.url().as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent"
        );
        assert_eq!(req.headers()["x-goog-api-key"], "gk");
        assert!(req.url().query().is_none(), "key must not be in the URL");
        let body: Value = serde_json::from_slice(req.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body["contents"][0]["parts"][1]["inline_data"]["data"],
            "AQID"
        );
        assert!(body["contents"][0]["parts"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"de\""));
    }

    #[test]
    fn parses() {
        let v = json!({"candidates":[{"content":{"parts":[{"text":" Hallo "},{"text":"Welt "}]}}]});
        assert_eq!(parse_response(&v).unwrap(), "Hallo Welt");
        assert_eq!(parse_response(&json!({"candidates":[{}]})).unwrap(), "");
        assert!(parse_response(&json!({})).is_err());
    }
}
