//! Configuration error types.

/// Errors that can occur when loading, saving, or parsing configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Failed to read the config file from disk.
    #[error("failed to read config: {0}")]
    ReadError(#[source] std::io::Error),

    /// Failed to write the config file to disk.
    #[error("failed to write config: {0}")]
    WriteError(#[source] std::io::Error),

    /// Failed to parse RON content.
    #[error("failed to parse config: {0}")]
    ParseError(#[source] ron::error::SpannedError),

    /// Failed to serialize config to RON.
    #[error("failed to serialize config: {0}")]
    SerializeError(#[source] ron::Error),
}
