//! Platform-independent core of blobtype. Everything here is testable without
//! a GUI, a microphone or Windows.

pub mod audio;
pub mod config;
pub mod error;
pub mod live;
pub mod providers;
pub mod settings;

pub use error::SttError;
