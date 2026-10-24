//! Shared helpers for the `/audio/transcriptions` multipart API that Groq,
//! OpenAI, Mistral and self-hosted whisper servers all implement.

use reqwest::multipart::{Form, Part};

use super::{check_status, http_client, Settings};
use crate::error::SttError;

/// Plain text fields of the multipart body (the audio part is added separately).
pub(crate) type Fields = Vec<(&'static str, String)>;

pub(crate) fn build_form(fields: &Fields, wav: Vec<u8>) -> Form {
    let mut form = Form::new();
    for (k, v) in fields {
        form = form.text(*k, v.clone());
    }
    let part = Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .expect("static mime");
    form.part("file", part)
}

pub(crate) fn build_request(
    client: &reqwest::Client,
    s: &Settings,
    fields: &Fields,
    wav: Vec<u8>,
) -> reqwest::RequestBuilder {
    let mut req = client
        .post(format!("{}/audio/transcriptions", s.base_url))
        .multipart(build_form(fields, wav));
    if !s.api_key.is_empty() {
        req = req.bearer_auth(&s.api_key);
    }
    req
}

pub(crate) async fn send(
    provider: &'static str,
    s: &Settings,
    fields: &Fields,
    wav: Vec<u8>,
) -> Result<String, SttError> {
    let client = http_client();
    let resp = build_request(&client, s, fields, wav).send().await?;
    let resp = check_status(provider, resp).await?;
    let body = resp.text().await?;
    parse_text(provider, &body)
}

/// `{"text": "..."}` (or a bare string for `response_format=text`).
pub(crate) fn parse_text(provider: &'static str, body: &str) -> Result<String, SttError> {
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) => v
            .get("text")
            .and_then(|t| t.as_str())
            .map(|t| t.trim().to_string())
            .ok_or(SttError::BadResponse(provider)),
        Err(_) if !body.trim().is_empty() && !body.trim_start().starts_with('{') => {
            Ok(body.trim().to_string())
        }
        Err(_) => Err(SttError::BadResponse(provider)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_and_plain() {
        assert_eq!(parse_text("x", r#"{"text":" hi "}"#).unwrap(), "hi");
        assert_eq!(parse_text("x", "plain\n").unwrap(), "plain");
        assert!(parse_text("x", r#"{"nope":1}"#).is_err());
        assert!(parse_text("x", "").is_err());
    }
}
