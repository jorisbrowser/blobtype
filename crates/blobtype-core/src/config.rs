//! `config.toml` handling: schema, defaults, commented template, key lookup.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::SttError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Batch,
    Live,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    Type,
    Paste,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SttConfig {
    /// groq | openai | deepgram | assemblyai | elevenlabs | mistral | google | openai-compatible
    pub provider: String,
    /// Empty = provider default.
    pub model: String,
    /// Empty = provider default. Required for `openai-compatible`.
    pub base_url: String,
    /// Name of the env var holding the key. Empty = provider default.
    pub api_key_env: String,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            provider: "groq".into(),
            model: String::new(),
            base_url: String::new(),
            api_key_env: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LiveConfig {
    /// openai | deepgram
    pub provider: String,
    pub model: String,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            model: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DiscordConfig {
    /// Press `key` when recording starts and stops (bound to "Toggle Mute" in Discord).
    pub mute: bool,
    /// F1 .. F24.
    pub key: String,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            mute: false,
            key: "F24".into(),
        }
    }
}

/// Windows virtual-key code for an F-key name ("F1" .. "F24", any case).
pub fn function_key_code(name: &str) -> Option<u16> {
    let n: u16 = name.trim().strip_prefix(['F', 'f'])?.parse().ok()?;
    // VK_F1 = 0x70 .. VK_F24 = 0x87
    (1..=24).contains(&n).then(|| 0x6F + n)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub hotkey: String,
    pub cancel_hotkey: String,
    /// Unregister the hotkey while a fullscreen app (e.g. a game) has focus.
    pub pause_in_fullscreen: bool,
    /// Types the last transcript again. Empty = off.
    pub last_hotkey: String,
    pub mode: Mode,
    pub output: Output,
    /// "auto" or an ISO-639-1 code such as "en", "de".
    pub language: String,
    /// Substring of the input device name; empty = system default.
    pub microphone: String,
    pub sounds: bool,
    pub max_recording_secs: u32,
    pub stt: SttConfig,
    pub live: LiveConfig,
    pub discord: DiscordConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Space".into(),
            cancel_hotkey: "Escape".into(),
            pause_in_fullscreen: true,
            last_hotkey: "Ctrl+Shift+Space".into(),
            mode: Mode::Batch,
            output: Output::Type,
            language: "auto".into(),
            microphone: String::new(),
            sounds: true,
            max_recording_secs: 120,
            stt: SttConfig::default(),
            live: LiveConfig::default(),
            discord: DiscordConfig::default(),
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, SttError> {
        let cfg: Config =
            toml::from_str(text).map_err(|e| SttError::Config(format!("config.toml: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), SttError> {
        if self.hotkey.trim().is_empty() {
            return Err(SttError::Config("config.toml: hotkey is empty".into()));
        }
        if self.max_recording_secs == 0 || self.max_recording_secs > 1800 {
            return Err(SttError::Config(
                "config.toml: max_recording_secs must be 1..=1800".into(),
            ));
        }
        if function_key_code(&self.discord.key).is_none() {
            return Err(SttError::Config(format!(
                "config.toml: discord.key \"{}\" must be F1 .. F24",
                self.discord.key
            )));
        }
        Ok(())
    }

    /// `None` when language is "auto"/empty.
    pub fn language_code(&self) -> Option<String> {
        let l = self.language.trim();
        if l.is_empty() || l.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(l.to_string())
        }
    }
}

/// Default env var for each provider's API key.
pub fn default_key_env(provider: &str) -> &'static str {
    match provider {
        "groq" => "GROQ_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "deepgram" => "DEEPGRAM_API_KEY",
        "assemblyai" => "ASSEMBLYAI_API_KEY",
        "elevenlabs" => "ELEVENLABS_API_KEY",
        "mistral" => "MISTRAL_API_KEY",
        "google" => "GEMINI_API_KEY",
        "openai-compatible" => "OPENAI_COMPATIBLE_API_KEY",
        _ => "STT_API_KEY",
    }
}

/// Look a key up through `getter` (normally `std::env::var(..).ok()`).
/// Empty values count as unset.
pub fn lookup_key(
    provider: &str,
    api_key_env: &str,
    getter: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let name = if api_key_env.trim().is_empty() {
        default_key_env(provider)
    } else {
        api_key_env.trim()
    };
    let nonempty = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    nonempty(getter(name)).or_else(|| {
        if provider == "google" && api_key_env.trim().is_empty() {
            nonempty(getter("GOOGLE_API_KEY"))
        } else {
            None
        }
    })
}

pub fn env_getter(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// `%APPDATA%/blobtype` on Windows, `~/.config/blobtype` elsewhere.
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.unwrap_or_else(|| PathBuf::from(".")).join("blobtype")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Load `.env` files (config dir first, then CWD). Existing env vars win.
pub fn load_dotenv() {
    let _ = dotenvy::from_path(config_dir().join(".env"));
    let _ = dotenvy::dotenv();
}

/// Read the config, creating the commented default file on first run.
pub fn load_or_create(path: &Path) -> Result<Config, SttError> {
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| SttError::Config(format!("cannot create {}: {e}", dir.display())))?;
        }
        std::fs::write(path, DEFAULT_CONFIG_TOML)
            .map_err(|e| SttError::Config(format!("cannot write {}: {e}", path.display())))?;
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| SttError::Config(format!("cannot read {}: {e}", path.display())))?;
    Config::parse(&text)
}

pub const DEFAULT_CONFIG_TOML: &str = include_str!("../../../config.example.toml");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_parses_and_equals_defaults() {
        assert_eq!(
            Config::parse(DEFAULT_CONFIG_TOML).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn partial_config_fills_defaults() {
        let c = Config::parse("mode = \"live\"\n[stt]\nprovider = \"openai\"").unwrap();
        assert_eq!(c.mode, Mode::Live);
        assert_eq!(c.stt.provider, "openai");
        assert_eq!(c.hotkey, "Ctrl+Space");
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Config::parse("mode = \"sometimes\"").is_err());
        assert!(Config::parse("max_recording_secs = 0").is_err());
        assert!(Config::parse("hotkey = \"\"").is_err());
        assert!(Config::parse("output = 5").is_err());
        assert!(Config::parse("[discord]\nkey = \"F25\"").is_err());
    }

    #[test]
    fn function_keys() {
        assert_eq!(function_key_code("F1"), Some(0x70));
        assert_eq!(function_key_code(" f24 "), Some(0x87));
        assert_eq!(function_key_code("F0"), None);
        assert_eq!(function_key_code("F25"), None);
        assert_eq!(function_key_code("Space"), None);
        assert_eq!(function_key_code(""), None);
    }

    #[test]
    fn language_auto() {
        let mut c = Config::default();
        assert_eq!(c.language_code(), None);
        c.language = "de".into();
        assert_eq!(c.language_code().as_deref(), Some("de"));
    }

    #[test]
    fn key_lookup() {
        let get = |n: &str| match n {
            "GROQ_API_KEY" => Some(" k1 ".to_string()),
            "MY_KEY" => Some("k2".to_string()),
            "EMPTY" => Some("  ".to_string()),
            "GOOGLE_API_KEY" => Some("g".to_string()),
            _ => None,
        };
        assert_eq!(lookup_key("groq", "", get).as_deref(), Some("k1"));
        assert_eq!(lookup_key("groq", "MY_KEY", get).as_deref(), Some("k2"));
        assert_eq!(lookup_key("groq", "EMPTY", get), None);
        assert_eq!(lookup_key("openai", "", get), None);
        assert_eq!(lookup_key("google", "", get).as_deref(), Some("g"));
    }
}
