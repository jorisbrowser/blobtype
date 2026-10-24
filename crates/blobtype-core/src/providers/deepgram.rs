//! Deepgram pre-recorded: `POST /v1/listen` with raw audio body, `Authorization: Token`.

use async_trait::async_trait;

use super::{check_status, http_client, Provider, Settings};
use crate::error::SttError;

pub struct Deepgram(Settings);

impl Deepgram {
    pub fn new(s: Settings) -> Self {
        Self(s)
    }

    pub(crate) fn query(&self) -> Vec<(&'static str, String)> {
        let mut q = vec![
            ("model", self.0.model.clone()),
            ("smart_format", "true".to_string()),
        ];
        match &self.0.language {
            Some(l) => q.push(("language", l.clone())),
            None => q.push(("detect_language", "true".to_string())),
        }
        q
    }

    pub(crate) fn request(
        &self,
        client: &reqwest::Client,
        wav: Vec<u8>,
    ) -> reqwest::RequestBuilder {
        client
            .post(format!("{}/listen", self.0.base_url))
            .query(&self.query())
            .header("Authorization", format!("Token {}", self.0.api_key))
            .header("Content-Type", "audio/wav")
            .body(wav)
    }
}

pub(crate) fn parse_response(body: &str) -> Result<String, SttError> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|_| SttError::BadResponse("deepgram"))?;
    v.pointer("/results/channels/0/alternatives/0/transcript")
        .and_then(|t| t.as_str())
        .map(|t| t.trim().to_string())
        .ok_or(SttError::BadResponse("deepgram"))
}

#[async_trait]
impl Provider for Deepgram {
    fn name(&self) -> &'static str {
        "deepgram"
    }
    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, SttError> {
        let resp = self.request(&http_client(), wav).send().await?;
        let body = check_status("deepgram", resp).await?.text().await?;
        parse_response(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dg(lang: Option<&str>) -> Deepgram {
        Deepgram::new(Settings {
            api_key: "dk".into(),
            model: "nova-3".into(),
            base_url: "https://api.deepgram.com/v1".into(),
            language: lang.map(str::to_string),
        })
    }

    #[test]
    fn request_shape() {
        let req = dg(Some("de"))
            .request(&reqwest::Client::new(), vec![1])
            .build()
            .unwrap();
        assert_eq!(req.url().path(), "/v1/listen");
        let q = req.url().query().unwrap();
        assert!(q.contains("model=nova-3") && q.contains("language=de"));
        assert_eq!(req.headers()["authorization"], "Token dk");
        assert_eq!(req.headers()["content-type"], "audio/wav");
    }

    #[test]
    fn auto_language_detects() {
        let req = dg(None)
            .request(&reqwest::Client::new(), vec![1])
            .build()
            .unwrap();
        assert!(req.url().query().unwrap().contains("detect_language=true"));
    }

    #[test]
    fn parses_response() {
        let body = r#"{"results":{"channels":[{"alternatives":[{"transcript":" hi there "}]}]}}"#;
        assert_eq!(parse_response(body).unwrap(), "hi there");
        assert!(parse_response("{}").is_err());
    }
}
