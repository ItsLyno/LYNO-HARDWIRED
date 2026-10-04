use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("integrity check failed: {0}")]
    Integrity(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("cancelled")]
    Cancelled,
    #[error("invalid manifest: {0}")]
    Manifest(String),
    /// Publishing a build: a check failed or the host refused.
    #[error("{0}")]
    Release(String),
    /// The Nexus API answered with an error status.
    #[error("Nexus: HTTP {status} {message}")]
    Nexus { status: u16, message: String },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }
}
