use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path:?}: {source}")]
    Io {
        path: Option<PathBuf>,
        #[source]
        source: std::io::Error,
    },
    #[error("parse error: {0}")]
    Parse(String),
    #[error("{agent} does not support {operation}")]
    Unsupported { agent: String, operation: String },
    #[error("already exists: {0}")]
    Exists(String),
    #[error("ledger error: {0}")]
    Ledger(#[from] rusqlite::Error),
    #[error("mass delete refused: plan removes {planned} entries ({total} items in ledger)")]
    MassDelete { planned: usize, total: usize },
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Io { path: None, source }
    }
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: Some(path.into()),
            source,
        }
    }

    pub fn unsupported(agent: impl Into<String>, operation: impl Into<String>) -> Self {
        Error::Unsupported {
            agent: agent.into(),
            operation: operation.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
