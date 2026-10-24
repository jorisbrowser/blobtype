//! Batch speech-to-text providers. One file per provider; to add one,
//! implement [`Provider`], register it in [`create`] and [`PROVIDERS`].

use async_trait::async_trait;
use std::time::Duration;

use crate::config::{default_key_env, lookup_key, SttConfig};
use crate::error::{short_message, SttError};

mod assemblyai;
mod deepgram;
mod elevenlabs;
mod google;
mod groq;
mod mistral;
mod openai;
mod openai_compat;
mod openai_style;

pub use assemblyai::AssemblyAi;
pub use deepgram::Deepgram;
pub use elevenlabs::ElevenLabs;
pub use google::Google;
pub use groq::Groq;
pub use mistral::Mistral;
pub use openai::OpenAi;
pub use openai_compat::OpenAiCompatible;

pub const PROVIDERS: &[&str] = &[
    "groq",
    "openai",
    "deepgram",
    "assemblyai",
    "elevenlabs",
    "mistral",
    "google",
    "openai-compatible",
];

/// Everything a provider needs for one transcription.
#[derive(Debug, Clone)]
pub struct Settings {
    pub api_key: String,
    /// Already resolved: never empty.
    pub model: String,
    /// Already resolved: never empty, no trailing slash.
    pub base_url: String,
    /// ISO-639-1 or `None` for auto-detect.
    pub language: Option<String>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    /// `wav` is a complete 16 kHz mono PCM16 WAV file.
    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, SttError>;
}

pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(90))
        .build()
        .expect("reqwest client")
}

/// Turn a non-2xx response into an [`SttError::Api`] with a short message.
pub(crate) async fn check_status(
    provider: &'static str,
    resp: reqwest::Response,
) -> Result<reqwest::Response, SttError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            // Common shapes: {"error":{"message":..}}, {"error":".."}, {"detail":{"message":..}}
            v.pointer("/error/message")
                .or_else(|| v.get("error"))
                .or_else(|| v.pointer("/detail/message"))
                .or_else(|| v.get("detail"))
                .or_else(|| v.get("err_msg"))
                .and_then(|m| m.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| short_message(&body));
    Err(SttError::Api {
        provider,
        status: status.as_u16(),
        message: short_message(&message),
    })
}

/// Default model for a provider (batch).
pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "groq" => "whisper-large-v3-turbo",
        "openai" => "gpt-transcribe",
        "deepgram" => "nova-3",
        "assemblyai" => "",
        "elevenlabs" => "scribe_v2",
        "mistral" => "voxtral-mini-latest",
        "google" => "gemini-3.8-flash",
        "openai-compatible" => "whisper-1",
        _ => "",
    }
}

pub fn default_base_url(provider: &str) -> &'static str {
    match provider {
        "groq" => "https://api.groq.com/openai/v1",
        "openai" => "https://api.openai.com/v1",
        "deepgram" => "https://api.deepgram.com/v1",
        "assemblyai" => "https://api.assemblyai.com/v2",
        "elevenlabs" => "https://api.elevenlabs.io/v1",
        "mistral" => "https://api.mistral.ai/v1",
        "google" => "https://generativelanguage.googleapis.com/v1beta",
        _ => "",
    }
}

/// Resolve config + key lookup into [`Settings`].
pub fn resolve_settings(
    cfg: &SttConfig,
    language: Option<String>,
    getter: impl Fn(&str) -> Option<String>,
) -> Result<Settings, SttError> {
    let provider = cfg.provider.trim().to_ascii_lowercase();
    if !PROVIDERS.contains(&provider.as_str()) {
        return Err(SttError::Config(format!(
            "Unknown provider \"{}\" (use one of: {})",
            cfg.provider,
            PROVIDERS.join(", ")
        )));
    }
    let base_url = if cfg.base_url.trim().is_empty() {
        default_base_url(&provider).to_string()
    } else {
        cfg.base_url.trim().to_string()
    };
    if base_url.is_empty() {
        return Err(SttError::Config(
            "provider \"openai-compatible\" needs stt.base_url".into(),
        ));
    }
    let model = if cfg.model.trim().is_empty() {
        default_model(&provider).to_string()
    } else {
        cfg.model.trim().to_string()
    };
    let key = lookup_key(&provider, &cfg.api_key_env, getter);
    let api_key = match key {
        Some(k) => k,
        // Self-hosted servers frequently need no key.
        None if provider == "openai-compatible" => String::new(),
        None => {
            let env = if cfg.api_key_env.trim().is_empty() {
                default_key_env(&provider).to_string()
            } else {
                cfg.api_key_env.trim().to_string()
            };
            return Err(SttError::MissingKey { env });
        }
    };
    Ok(Settings {
        api_key,
        model,
        base_url: base_url.trim_end_matches('/').to_string(),
        language,
    })
}

pub fn create(
    cfg: &SttConfig,
    language: Option<String>,
    getter: impl Fn(&str) -> Option<String>,
) -> Result<Box<dyn Provider>, SttError> {
    let s = resolve_settings(cfg, language, getter)?;
    Ok(match cfg.provider.trim().to_ascii_lowercase().as_str() {
        "groq" => Box::new(Groq::new(s)),
        "openai" => Box::new(OpenAi::new(s)),
        "deepgram" => Box::new(Deepgram::new(s)),
        "assemblyai" => Box::new(AssemblyAi::new(s)),
        "elevenlabs" => Box::new(ElevenLabs::new(s)),
        "mistral" => Box::new(Mistral::new(s)),
        "google" => Box::new(Google::new(s)),
        "openai-compatible" => Box::new(OpenAiCompatible::new(s)),
        _ => unreachable!("validated in resolve_settings"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(n: &str) -> Option<String> {
        Some(format!("key-for-{n}"))
    }

    #[test]
    fn every_provider_can_be_created() {
        for p in PROVIDERS {
            let cfg = SttConfig {
                provider: p.to_string(),
                base_url: "http://localhost:1/v1".into(),
                ..Default::default()
            };
            let prov = create(&cfg, None, keys).unwrap();
            assert_eq!(prov.name(), *p);
        }
    }

    #[test]
    fn missing_key_names_env_var() {
        let cfg = SttConfig::default();
        match resolve_settings(&cfg, None, |_| None) {
            Err(SttError::MissingKey { env }) => assert_eq!(env, "GROQ_API_KEY"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn compatible_needs_base_url_but_no_key() {
        let mut cfg = SttConfig {
            provider: "openai-compatible".into(),
            ..Default::default()
        };
        assert!(resolve_settings(&cfg, None, |_| None).is_err());
        cfg.base_url = "http://localhost:8000/v1/".into();
        let s = resolve_settings(&cfg, None, |_| None).unwrap();
        assert_eq!(s.base_url, "http://localhost:8000/v1");
        assert_eq!(s.api_key, "");
    }

    #[test]
    fn unknown_provider_rejected() {
        let cfg = SttConfig {
            provider: "nope".into(),
            ..Default::default()
        };
        assert!(matches!(
            resolve_settings(&cfg, None, keys),
            Err(SttError::Config(_))
        ));
    }

    #[test]
    fn model_override_wins() {
        let cfg = SttConfig {
            model: "whisper-large-v3".into(),
            ..Default::default()
        };
        assert_eq!(
            resolve_settings(&cfg, None, keys).unwrap().model,
            "whisper-large-v3"
        );
    }
}
