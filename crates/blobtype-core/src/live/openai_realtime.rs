//! OpenAI Realtime API in transcription mode.
//!
//! Session setup follows the working implementation verified against the live
//! API (`wss://api.openai.com/v1/realtime?intent=transcription`). This is a
//! fast-moving beta surface: re-check the schema if OpenAI changes it.

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use super::LiveEvent;
use crate::audio::{pcm16_to_le_bytes, resample_linear, to_pcm16, STT_SAMPLE_RATE};
use crate::error::SttError;

pub const WS_URL: &str = "wss://api.openai.com/v1/realtime?intent=transcription";
pub const DEFAULT_MODEL: &str = "gpt-live-transcribe";
const WS_SAMPLE_RATE: u32 = 24_000;
/// After the commit: give up if nothing arrives for this long.
const DRAIN_IDLE: Duration = Duration::from_secs(4);
const DRAIN_MAX: Duration = Duration::from_secs(20);

/// `gpt-live-transcribe` streams deltas while audio arrives and rejects
/// `turn_detection`; the other models need server VAD to produce incremental
/// results per utterance.
fn is_live_model(model: &str) -> bool {
    model.starts_with("gpt-live-transcribe")
}

pub(crate) fn session_update(model: &str, language: Option<&str>) -> Value {
    let mut transcription = json!({ "model": model });
    if is_live_model(model) {
        transcription["delay"] = json!("low");
    }
    if let Some(l) = language {
        transcription["language"] = json!(l);
    }
    let turn_detection = if is_live_model(model) {
        Value::Null
    } else {
        json!({ "type": "server_vad" })
    };
    json!({
        "type": "session.update",
        "session": {
            "type": "transcription",
            "audio": { "input": {
                "format": { "type": "audio/pcm", "rate": WS_SAMPLE_RATE },
                "transcription": transcription,
                "turn_detection": turn_detection,
            }}
        }
    })
}

pub(crate) fn append_event(samples_16k: &[f32]) -> Value {
    let resampled = resample_linear(samples_16k, STT_SAMPLE_RATE, WS_SAMPLE_RATE);
    let bytes = pcm16_to_le_bytes(&to_pcm16(&resampled));
    json!({
        "type": "input_audio_buffer.append",
        "audio": base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RtEvent {
    Delta { item_id: String, delta: String },
    Completed { item_id: String, transcript: String },
    Error { code: String, message: String },
    Other,
}

pub(crate) fn parse_event(text: &str) -> RtEvent {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return RtEvent::Other;
    };
    let s = |p: &str| {
        v.pointer(p)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("conversation.item.input_audio_transcription.delta") => RtEvent::Delta {
            item_id: s("/item_id"),
            delta: s("/delta"),
        },
        Some("conversation.item.input_audio_transcription.completed") => RtEvent::Completed {
            item_id: s("/item_id"),
            transcript: s("/transcript"),
        },
        Some("error") => RtEvent::Error {
            code: s("/error/code"),
            message: s("/error/message"),
        },
        _ => RtEvent::Other,
    }
}

/// Decides what to type for incoming events. Pure, so it is unit-tested.
#[derive(Default)]
pub(crate) struct TextAssembler {
    current_item: Option<String>,
    item_has_deltas: bool,
    typed_any: bool,
}

impl TextAssembler {
    fn start_item(&mut self, item_id: &str) -> String {
        if self.current_item.as_deref() == Some(item_id) {
            return String::new();
        }
        self.current_item = Some(item_id.to_string());
        self.item_has_deltas = false;
        // A new utterance is separated from the previous one by a space.
        if self.typed_any {
            " ".into()
        } else {
            String::new()
        }
    }

    pub fn on_event(&mut self, ev: &RtEvent) -> Option<String> {
        match ev {
            RtEvent::Delta { item_id, delta } if !delta.is_empty() => {
                let mut out = self.start_item(item_id);
                out.push_str(delta);
                self.item_has_deltas = true;
                self.typed_any = true;
                Some(out)
            }
            RtEvent::Completed {
                item_id,
                transcript,
            } => {
                let mut out = self.start_item(item_id);
                let typed_already = self.item_has_deltas;
                self.current_item = None;
                self.item_has_deltas = false;
                if typed_already || transcript.trim().is_empty() {
                    return None;
                }
                out.push_str(transcript.trim());
                self.typed_any = true;
                Some(out)
            }
            _ => None,
        }
    }
}

pub async fn run(
    api_key: &str,
    model: &str,
    language: Option<String>,
    mut audio_rx: UnboundedReceiver<Vec<f32>>,
    events: UnboundedSender<LiveEvent>,
) -> Result<(), SttError> {
    let model = if model.is_empty() {
        DEFAULT_MODEL
    } else {
        model
    };
    let mut request = WS_URL
        .into_client_request()
        .map_err(|e| SttError::Network(e.to_string()))?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|_| SttError::Config("API key contains invalid characters".into()))?,
    );
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| SttError::Network(e.to_string()))?;
    let (mut write, mut read) = ws.split();

    write
        .send(Message::Text(
            session_update(model, language.as_deref()).to_string(),
        ))
        .await
        .map_err(|e| SttError::Network(e.to_string()))?;

    let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();
    let sender = tokio::spawn(async move {
        while let Some(block) = audio_rx.recv().await {
            if write
                .send(Message::Text(append_event(&block).to_string()))
                .await
                .is_err()
            {
                return;
            }
        }
        let _ = write
            .send(Message::Text(
                json!({ "type": "input_audio_buffer.commit" }).to_string(),
            ))
            .await;
        // No Close frame: it can race the in-flight "completed" event.
        let _ = done_tx.send(());
        // Keep the write half alive until the receiver is done.
        std::future::pending::<()>().await;
    });

    let mut asm = TextAssembler::default();
    let mut audio_ended = false;
    let drain_deadline = tokio::time::sleep(Duration::from_secs(60 * 60 * 24));
    tokio::pin!(drain_deadline);
    let mut result = Ok(());

    loop {
        let idle = tokio::time::sleep(if audio_ended {
            DRAIN_IDLE
        } else {
            Duration::from_secs(60 * 60)
        });
        tokio::select! {
            _ = &mut done_rx, if !audio_ended => {
                audio_ended = true;
                drain_deadline.as_mut().reset(tokio::time::Instant::now() + DRAIN_MAX);
            }
            _ = &mut drain_deadline => break,
            _ = idle => break,
            msg = read.next() => {
                let Some(msg) = msg else { break };
                let text = match msg {
                    Ok(Message::Text(t)) => t,
                    Ok(Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(e) => {
                        result = Err(SttError::Network(e.to_string()));
                        break;
                    }
                };
                let ev = parse_event(&text);
                if let Some(piece) = asm.on_event(&ev) {
                    let _ = events.send(LiveEvent::Text(piece));
                }
                match ev {
                    RtEvent::Completed { .. } if audio_ended => break,
                    RtEvent::Error { code, message } => {
                        // Committing an empty buffer (e.g. VAD already consumed it) is harmless.
                        if audio_ended && code.contains("empty") {
                            break;
                        }
                        result = Err(SttError::Api {
                            provider: "openai",
                            status: 0,
                            message,
                        });
                        break;
                    }
                    _ => {}
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
    fn session_for_live_model() {
        let v = session_update("gpt-live-transcribe", Some("de"));
        let input = &v["session"]["audio"]["input"];
        assert_eq!(v["type"], "session.update");
        assert_eq!(v["session"]["type"], "transcription");
        assert_eq!(input["format"]["rate"], 24000);
        assert_eq!(input["transcription"]["delay"], "low");
        assert_eq!(input["transcription"]["language"], "de");
        assert!(input["turn_detection"].is_null());
    }

    #[test]
    fn session_for_vad_model() {
        let v = session_update("gpt-4o-transcribe", None);
        let input = &v["session"]["audio"]["input"];
        assert_eq!(input["turn_detection"]["type"], "server_vad");
        assert!(input["transcription"].get("delay").is_none());
        assert!(input["transcription"].get("language").is_none());
    }

    #[test]
    fn append_resamples_to_24k() {
        let v = append_event(&vec![0.0; 1600]); // 100 ms @16k
        let b64 = v["audio"].as_str().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        assert_eq!(bytes.len(), 2400 * 2);
    }

    #[test]
    fn parses_events() {
        assert_eq!(
            parse_event(
                r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"a","delta":"hi"}"#
            ),
            RtEvent::Delta {
                item_id: "a".into(),
                delta: "hi".into()
            }
        );
        assert_eq!(
            parse_event(r#"{"type":"error","error":{"code":"x","message":"boom"}}"#),
            RtEvent::Error {
                code: "x".into(),
                message: "boom".into()
            }
        );
        assert_eq!(parse_event("garbage"), RtEvent::Other);
        assert_eq!(parse_event(r#"{"type":"session.updated"}"#), RtEvent::Other);
    }

    fn delta(id: &str, d: &str) -> RtEvent {
        RtEvent::Delta {
            item_id: id.into(),
            delta: d.into(),
        }
    }
    fn done(id: &str, t: &str) -> RtEvent {
        RtEvent::Completed {
            item_id: id.into(),
            transcript: t.into(),
        }
    }

    #[test]
    fn assembler_types_deltas_once() {
        let mut a = TextAssembler::default();
        assert_eq!(a.on_event(&delta("1", "Hello")).as_deref(), Some("Hello"));
        assert_eq!(a.on_event(&delta("1", " world")).as_deref(), Some(" world"));
        assert_eq!(a.on_event(&done("1", "Hello world")), None);
        // second utterance gets a separating space
        assert_eq!(a.on_event(&delta("2", "Again")).as_deref(), Some(" Again"));
    }

    #[test]
    fn assembler_falls_back_to_completed() {
        let mut a = TextAssembler::default();
        assert_eq!(a.on_event(&done("1", " One. ")).as_deref(), Some("One."));
        assert_eq!(a.on_event(&done("2", "Two.")).as_deref(), Some(" Two."));
        assert_eq!(a.on_event(&done("3", "  ")), None);
    }
}
