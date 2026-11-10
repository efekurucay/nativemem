use nativemem_core::error::Error as CoreError;

#[derive(Debug, thiserror::Error)]
pub enum HubError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("{0}")]
    Invalid(String),
    #[error("server {0:?} already exists")]
    Exists(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Parse(String),
    #[error("secret store: {0}")]
    SecretStore(String),
    #[error("{0}")]
    Busy(String),
}

pub type Result<T> = std::result::Result<T, HubError>;

impl HubError {
    /// The code of the CLI error document (`docs/desktop.md`).
    pub fn code(&self) -> &'static str {
        match self {
            HubError::Core(CoreError::Io { .. }) => "io",
            HubError::Core(CoreError::Parse(_)) | HubError::Parse(_) => "parse",
            HubError::Core(_) => "io",
            HubError::Invalid(_) => "invalid",
            HubError::Exists(_) => "exists",
            HubError::NotFound(_) => "not_found",
            HubError::SecretStore(_) => "io",
            HubError::Busy(_) => "busy",
        }
    }
}
