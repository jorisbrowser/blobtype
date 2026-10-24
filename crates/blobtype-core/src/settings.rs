//! Editing the user's settings from the GUI: API keys go to `.env`, everything
//! else goes to `config.toml` (comments and unknown keys are preserved).

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::config::{self, Config, Mode, Output};
use crate::error::SttError;
use crate::live::LIVE_PROVIDERS;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: &'static str,
    pub label: &'static str,
    /// Where to create a key; empty for self-hosted servers.
    pub key_url: &'static str,
    pub note: &'static str,
    pub live: bool,
    pub self_hosted: bool,
}

const fn p(
    id: &'static str,
    label: &'static str,
    key_url: &'static str,
    note: &'static str,
) -> ProviderInfo {
    ProviderInfo {
        id,
        label,
        key_url,
        note,
        live: false,
        self_hosted: false,
    }
}

/// Providers offered in the settings window, in display order.
pub fn providers() -> Vec<ProviderInfo> {
    let mut list = vec![
        p(
            "groq",
            "Groq",
            "https://console.groq.com/keys",
            "Very fast, with a free tier. Recommended.",
        ),
        p(
            "openai",
            "OpenAI",
            "https://platform.openai.com/api-keys",
            "Accurate. Supports live typing.",
        ),
        p(
            "deepgram",
            "Deepgram",
            "https://console.deepgram.com",
            "Fast. Supports live typing.",
        ),
        p(
            "elevenlabs",
            "ElevenLabs",
            "https://elevenlabs.io/app/settings/api-keys",
            "Strong across many languages.",
        ),
        p(
            "assemblyai",
            "AssemblyAI",
            "https://www.assemblyai.com/dashboard",
            "Accurate, a bit slower on short clips.",
        ),
        p(
            "mistral",
            "Mistral",
            "https://console.mistral.ai/api-keys",
            "Voxtral models, based in the EU.",
        ),
        p(
            "google",
            "Google Gemini",
            "https://aistudio.google.com/apikey",
            "Key from Google AI Studio.",
        ),
        p(
            "openai-compatible",
            "Self-hosted",
            "",
            "Your own Whisper server. Audio stays on your network.",
        ),
    ];
    for info in &mut list {
        info.live = LIVE_PROVIDERS.contains(&info.id);
        info.self_hosted = info.id == "openai-compatible";
    }
    list
}

pub fn provider(id: &str) -> Option<ProviderInfo> {
    providers().into_iter().find(|p| p.id == id)
}

/// Everything the settings window can change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiSettings {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub language: String,
    pub live: bool,
    pub paste: bool,
    pub hotkey: String,
    pub last_hotkey: String,
    pub microphone: String,
    pub sounds: bool,
    pub max_recording_secs: u32,
    pub discord_mute: bool,
    pub discord_key: String,
}

impl UiSettings {
    pub fn from_config(cfg: &Config) -> Self {
        let live = cfg.mode == Mode::Live;
        Self {
            provider: if live {
                cfg.live.provider.clone()
            } else {
                cfg.stt.provider.clone()
            },
            model: cfg.stt.model.clone(),
            base_url: cfg.stt.base_url.clone(),
            language: cfg.language.clone(),
            live,
            paste: cfg.output == Output::Paste,
            hotkey: cfg.hotkey.clone(),
            last_hotkey: cfg.last_hotkey.clone(),
            microphone: cfg.microphone.clone(),
            sounds: cfg.sounds,
            max_recording_secs: cfg.max_recording_secs,
            discord_mute: cfg.discord.mute,
            discord_key: cfg.discord.key.clone(),
        }
    }
}

/// The provider whose key the next recording needs.
pub fn active_provider(cfg: &Config) -> &str {
    match cfg.mode {
        Mode::Live => &cfg.live.provider,
        Mode::Batch => &cfg.stt.provider,
    }
}

/// Env var that holds the key for `provider`. A custom `api_key_env` only
/// applies to the batch provider it was configured for.
pub fn key_env_for(cfg: &Config, provider: &str) -> String {
    if cfg.mode == Mode::Batch
        && provider == cfg.stt.provider
        && !cfg.stt.api_key_env.trim().is_empty()
    {
        cfg.stt.api_key_env.trim().to_string()
    } else {
        config::default_key_env(provider).to_string()
    }
}

/// Whether the next recording would find a key. Self-hosted servers may not need one.
pub fn ready(cfg: &Config) -> bool {
    let p = active_provider(cfg);
    p == "openai-compatible"
        || config::lookup_key(p, &key_env_for(cfg, p), config::env_getter).is_some()
}

/// `…abcd` hint for a stored key, never the key itself.
pub fn key_hint(key: &str) -> String {
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

/// Set `name=value` in `.env` text, replacing an existing assignment.
pub fn upsert_env(text: &str, name: &str, value: &str) -> String {
    let line = format!("{name}={value}");
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|l| {
            let t = l
                .trim_start()
                .strip_prefix("export ")
                .unwrap_or(l.trim_start());
            if t.split('=').next().map(str::trim) == Some(name) && t.contains('=') {
                found = true;
                line.clone()
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(line);
    }
    lines.join("\n") + "\n"
}

/// Store a key in `<config dir>/.env` and make it effective for this process.
pub fn save_key(name: &str, value: &str) -> Result<(), SttError> {
    let value = value.trim();
    if value.is_empty()
        || value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '"' || c == '\'')
    {
        return Err(SttError::Config(
            "The API key contains invalid characters".into(),
        ));
    }
    let dir = config::config_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| SttError::Config(format!("cannot create {}: {e}", dir.display())))?;
    let path = dir.join(".env");
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, upsert_env(&old, name, value))
        .map_err(|e| SttError::Config(format!("cannot write {}: {e}", path.display())))?;
    // `load_dotenv` never overrides existing vars, so set it directly.
    std::env::set_var(name, value);
    Ok(())
}

/// Return `text` with the UI settings applied; everything else is untouched.
pub fn apply_settings(text: &str, s: &UiSettings) -> Result<String, SttError> {
    use toml_edit::value;
    let info = provider(&s.provider)
        .ok_or_else(|| SttError::Config(format!("Unknown provider \"{}\"", s.provider)))?;
    if s.live && !info.live {
        return Err(SttError::Config(format!(
            "{} does not support live typing",
            info.label
        )));
    }
    if info.self_hosted && s.base_url.trim().is_empty() {
        return Err(SttError::Config("Enter the address of your server".into()));
    }
    let mut doc: toml_edit::Document = text
        .parse()
        .map_err(|e| SttError::Config(format!("config.toml: {e}")))?;
    let provider_changed = doc["stt"]["provider"].as_str() != Some(s.provider.as_str());
    doc["hotkey"] = value(s.hotkey.trim());
    doc["last_hotkey"] = value(s.last_hotkey.trim());
    doc["mode"] = value(if s.live { "live" } else { "batch" });
    doc["output"] = value(if s.paste { "paste" } else { "type" });
    doc["language"] = value(s.language.trim());
    doc["microphone"] = value(s.microphone.trim());
    doc["sounds"] = value(s.sounds);
    doc["max_recording_secs"] = value(i64::from(s.max_recording_secs));
    doc["stt"]["provider"] = value(s.provider.as_str());
    doc["stt"]["model"] = value(s.model.trim());
    doc["stt"]["base_url"] = value(s.base_url.trim());
    if provider_changed {
        doc["stt"]["api_key_env"] = value("");
    }
    doc["discord"]["mute"] = value(s.discord_mute);
    doc["discord"]["key"] = value(s.discord_key.trim());
    if s.live {
        doc["live"]["provider"] = value(s.provider.as_str());
    }
    Ok(doc.to_string())
}

pub fn save_config(path: &Path, s: &UiSettings) -> Result<(), SttError> {
    config::load_or_create(path)?;
    let old = std::fs::read_to_string(path)
        .map_err(|e| SttError::Config(format!("cannot read {}: {e}", path.display())))?;
    let new = apply_settings(&old, s)?;
    Config::parse(&new)?;
    std::fs::write(path, new)
        .map_err(|e| SttError::Config(format!("cannot write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> UiSettings {
        UiSettings::from_config(&Config::default())
    }

    #[test]
    fn env_upsert() {
        assert_eq!(upsert_env("", "A", "1"), "A=1\n");
        assert_eq!(upsert_env("# c\nA=old\nB=2\n", "A", "1"), "# c\nA=1\nB=2\n");
        assert_eq!(upsert_env("B=2", "A", "1"), "B=2\nA=1\n");
        assert_eq!(upsert_env("export A=x", "A", "1"), "A=1\n");
    }

    #[test]
    fn hint_hides_key() {
        assert_eq!(key_hint("gsk_secret1234"), "…1234");
    }

    #[test]
    fn defaults_round_trip() {
        let out = apply_settings(config::DEFAULT_CONFIG_TOML, &ui()).unwrap();
        assert_eq!(Config::parse(&out).unwrap(), Config::default());
        assert!(out.contains("# blobtype configuration"));
    }

    #[test]
    fn applies_every_field() {
        let s = UiSettings {
            provider: "deepgram".into(),
            model: "nova-3".into(),
            base_url: String::new(),
            language: "de".into(),
            live: true,
            paste: true,
            hotkey: "Ctrl+Alt+D".into(),
            last_hotkey: "Ctrl+Alt+L".into(),
            microphone: "USB Mic".into(),
            sounds: false,
            max_recording_secs: 300,
            discord_mute: true,
            discord_key: "F13".into(),
        };
        let c = Config::parse(&apply_settings(config::DEFAULT_CONFIG_TOML, &s).unwrap()).unwrap();
        assert_eq!(c.mode, Mode::Live);
        assert_eq!(c.live.provider, "deepgram");
        assert_eq!(c.output, Output::Paste);
        assert_eq!(c.hotkey, "Ctrl+Alt+D");
        assert_eq!(c.last_hotkey, "Ctrl+Alt+L");
        assert_eq!(c.microphone, "USB Mic");
        assert!(!c.sounds);
        assert_eq!(c.max_recording_secs, 300);
        assert!(c.discord.mute);
        assert_eq!(c.discord.key, "F13");
        assert_eq!(UiSettings::from_config(&c), s);
    }

    #[test]
    fn adds_new_settings_to_older_configs() {
        let old = config::DEFAULT_CONFIG_TOML
            .split("[discord]")
            .next()
            .unwrap()
            // CI checks out with CRLF on Windows: no stray `\r` at the end.
            .trim_end()
            .replace("last_hotkey = \"Ctrl+Shift+Space\"", "");
        let s = UiSettings {
            discord_mute: true,
            last_hotkey: "Ctrl+Alt+L".into(),
            ..ui()
        };
        let c = Config::parse(&apply_settings(&old, &s).unwrap()).unwrap();
        assert!(c.discord.mute);
        assert_eq!(c.discord.key, "F24");
        assert_eq!(c.last_hotkey, "Ctrl+Alt+L");
    }

    #[test]
    fn provider_change_resets_custom_env() {
        let text = config::DEFAULT_CONFIG_TOML.replace("api_key_env = \"\"", "api_key_env = \"X\"");
        let same = apply_settings(&text, &ui()).unwrap();
        assert_eq!(Config::parse(&same).unwrap().stt.api_key_env, "X");
        let other = UiSettings {
            provider: "openai".into(),
            ..ui()
        };
        let changed = apply_settings(&text, &other).unwrap();
        assert_eq!(Config::parse(&changed).unwrap().stt.api_key_env, "");
    }

    #[test]
    fn rejects_invalid_combinations() {
        assert!(apply_settings(
            config::DEFAULT_CONFIG_TOML,
            &UiSettings { live: true, ..ui() }
        )
        .is_err());
        let hosted = UiSettings {
            provider: "openai-compatible".into(),
            ..ui()
        };
        assert!(apply_settings(config::DEFAULT_CONFIG_TOML, &hosted).is_err());
        assert!(apply_settings(
            config::DEFAULT_CONFIG_TOML,
            &UiSettings {
                provider: "nope".into(),
                ..ui()
            }
        )
        .is_err());
    }

    #[test]
    fn every_provider_has_a_key_page_or_is_self_hosted() {
        for p in providers() {
            assert!(
                p.self_hosted || p.key_url.starts_with("https://"),
                "{}",
                p.id
            );
        }
    }

    #[test]
    fn rejects_bad_keys() {
        assert!(save_key("A", "has space").is_err());
        assert!(save_key("A", "").is_err());
    }
}
