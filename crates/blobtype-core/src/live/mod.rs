//! Live (streaming) transcription over websockets. Text is emitted
//! incrementally as [`LiveEvent::Text`] pieces that are meant to be typed
//! one after the other.

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::config::{default_key_env, lookup_key, Config};
use crate::error::SttError;

mod deepgram;
mod openai_realtime;

pub const LIVE_PROVIDERS: &[&str] = &["openai", "deepgram"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEvent {
    /// A piece of text to type right now (already contains any spacing).
    Text(String),
}

/// 16 kHz mono f32 blocks in, [`LiveEvent`]s out. Returns once the audio
/// channel is closed (recording stopped) and the final text has been flushed.
pub async fn run(
    cfg: &Config,
    getter: impl Fn(&str) -> Option<String>,
    audio_rx: UnboundedReceiver<Vec<f32>>,
    events: UnboundedSender<LiveEvent>,
) -> Result<(), SttError> {
    let provider = cfg.live.provider.trim().to_ascii_lowercase();
    if !LIVE_PROVIDERS.contains(&provider.as_str()) {
        return Err(SttError::Config(format!(
            "Unknown live provider \"{}\" (use openai or deepgram)",
            cfg.live.provider
        )));
    }
    let api_key = lookup_key(&provider, "", getter).ok_or_else(|| SttError::MissingKey {
        env: default_key_env(&provider).to_string(),
    })?;
    let model = cfg.live.model.trim().to_string();
    let language = cfg.language_code();
    match provider.as_str() {
        "openai" => openai_realtime::run(&api_key, &model, language, audio_rx, events).await,
        _ => deepgram::run(&api_key, &model, language, audio_rx, events).await,
    }
}
