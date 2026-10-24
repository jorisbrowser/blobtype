//! Deepgram live streaming (`wss://api.deepgram.com/v1/listen`).

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use super::LiveEvent;
use crate::audio::{pcm16_to_le_bytes, to_pcm16, STT_SAMPLE_RATE};
use crate::error::SttError;

pub const DEFAULT_MODEL: &str = "nova-3";
const DRAIN_MAX: Duration = Duration::from_secs(8);

pub(crate) fn ws_url(model: &str, language: Option<&str>) -> String {
    // `multi` = automatic multilingual (code-switching) on nova-3.
    format!(
        "wss://api.deepgram.com/v1/listen?model={}&encoding=linear16&sample_rate={}&channels=1\
         &interim_results=false&smart_format=true&endpointing=300&language={}",
        if model.is_empty() {
            DEFAULT_MODEL
        } else {
            model
        },
        STT_SAMPLE_RATE,
        language.unwrap_or("multi"),
    )
}

/// Text of a final `Results` message, `None` for anything else.
pub(crate) fn final_text(msg: &str) -> Option<String> {
    let v: Value = serde_json::from_str(msg).ok()?;
    if v.get("type")?.as_str()? != "Results" || !v.get("is_final")?.as_bool()? {
        return None;
    }
    let t = v
        .pointer("/channel/alternatives/0/transcript")?
        .as_str()?
        .trim();
    (!t.is_empty()).then(|| t.to_string())
}

pub async fn run(
    api_key: &str,
    model: &str,
    language: Option<String>,
    mut audio_rx: UnboundedReceiver<Vec<f32>>,
    events: UnboundedSender<LiveEvent>,
) -> Result<(), SttError> {
    let mut request = ws_url(model, language.as_deref())
        .into_client_request()
        .map_err(|e| SttError::Network(e.to_string()))?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Token {api_key}"))
            .map_err(|_| SttError::Config("API key contains invalid characters".into()))?,
    );
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| SttError::Network(e.to_string()))?;
    let (mut write, mut read) = ws.split();

    let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();
    let sender = tokio::spawn(async move {
        while let Some(block) = audio_rx.recv().await {
            let bytes = pcm16_to_le_bytes(&to_pcm16(&block));
            if write.send(Message::Binary(bytes)).await.is_err() {
                return;
            }
        }
        let _ = write
            .send(Message::Text(json!({"type":"Finalize"}).to_string()))
            .await;
        let _ = write
            .send(Message::Text(json!({"type":"CloseStream"}).to_string()))
            .await;
        let _ = done_tx.send(());
        std::future::pending::<()>().await;
    });

    let mut first = true;
    let mut audio_ended = false;
    let drain = tokio::time::sleep(Duration::from_secs(60 * 60 * 24));
    tokio::pin!(drain);
    let mut result = Ok(());
    loop {
        tokio::select! {
            _ = &mut done_rx, if !audio_ended => {
                audio_ended = true;
                drain.as_mut().reset(tokio::time::Instant::now() + DRAIN_MAX);
            }
            _ = &mut drain => break,
            msg = read.next() => {
                let Some(msg) = msg else { break };
                match msg {
                    Ok(Message::Text(t)) => {
                        if let Some(text) = final_text(&t) {
                            let piece = if first { text } else { format!(" {text}") };
                            first = false;
                            let _ = events.send(LiveEvent::Text(piece));
                        }
                    }
                    // The server closes after CloseStream once everything is flushed.
                    Ok(Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(e) => {
                        result = Err(SttError::Network(e.to_string()));
                        break;
                    }
                }
            }
        }
    }
    sender.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_params() {
        let u = ws_url("", None);
        assert!(u.contains("model=nova-3") && u.contains("encoding=linear16"));
        assert!(u.contains("sample_rate=16000") && u.contains("language=multi"));
        assert!(ws_url("nova-2", Some("de")).contains("language=de"));
    }

    #[test]
    fn extracts_final_results_only() {
        let fin = r#"{"type":"Results","is_final":true,"channel":{"alternatives":[{"transcript":" hello "}]}}"#;
        let interim = r#"{"type":"Results","is_final":false,"channel":{"alternatives":[{"transcript":"hel"}]}}"#;
        let empty =
            r#"{"type":"Results","is_final":true,"channel":{"alternatives":[{"transcript":""}]}}"#;
        assert_eq!(final_text(fin).as_deref(), Some("hello"));
        assert_eq!(final_text(interim), None);
        assert_eq!(final_text(empty), None);
        assert_eq!(final_text(r#"{"type":"Metadata"}"#), None);
    }
}
