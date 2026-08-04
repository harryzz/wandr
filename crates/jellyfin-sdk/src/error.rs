use reqwest::StatusCode;

/// A crate-wide result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by this SDK.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Base URL could not be parsed or normalized.
    #[error("invalid base url: {0}")]
    InvalidBaseUrl(#[from] url::ParseError),

    /// HTTP client error (transport, timeout, TLS, etc).
    #[error(transparent)]
    Http(#[from] reqwest::Error),

    /// A non-success HTTP status with captured body (best-effort).
    #[error("request failed with status {status}: {body}")]
    Api {
        /// HTTP status code.
        status: StatusCode,
        /// Response body captured as UTF-8 text (lossy).
        body: String,
        /// Parsed Retry-After header in seconds, if present.
        retry_after_seconds: Option<u64>,
    },

    /// JSON decoding error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// I/O error (filesystem, etc).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Invalid configuration provided to the client builder.
    #[error("invalid configuration: {0}")]
    InvalidConfig(&'static str),

    /// A user-provided timecode string could not be parsed.
    #[error("invalid timecode: {0}")]
    InvalidTimecode(String),
}
