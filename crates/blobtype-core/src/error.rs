use thiserror::Error;

/// Errors surfaced to the user (shown as a toast in the blob), so the
/// messages are short and never contain secrets or audio.
#[derive(Debug, Error)]
pub enum SttError {
    #[error("No API key: set {env} (env var or .env)")]
    MissingKey { env: String },
    #[error("{0}")]
    Config(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("{provider} error {status}: {message}")]
    Api {
        provider: &'static str,
        status: u16,
        message: String,
    },
    #[error("Unexpected response from {0}")]
    BadResponse(&'static str),
    #[error("Timed out waiting for {0}")]
    Timeout(&'static str),
}

impl From<reqwest::Error> for SttError {
    fn from(e: reqwest::Error) -> Self {
        // Strip the URL: it may carry query-string credentials in the future.
        SttError::Network(e.without_url().to_string())
    }
}

/// Trim a (possibly huge) error body down to something toast-sized.
pub fn short_message(body: &str) -> String {
    let flat: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 160 {
        let cut: String = flat.chars().take(160).collect();
        format!("{cut}…")
    } else {
        flat
    }
}
