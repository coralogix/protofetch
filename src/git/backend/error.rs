use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitBackendError {
    #[error("Git error: {0}")]
    GitError(String),

    #[error("Object not found: {0}")]
    NotFound(String),

    #[error("Repository not found at {0}")]
    RepoNotFound(String),

    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Invalid reference: {0}")]
    InvalidRef(String),

    #[error("Command failed: {0}")]
    CommandFailed(String),

    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}

impl From<git2::Error> for GitBackendError {
    fn from(e: git2::Error) -> Self {
        match e.code() {
            git2::ErrorCode::NotFound => GitBackendError::NotFound(e.message().to_string()),
            git2::ErrorCode::Auth => GitBackendError::AuthError(e.message().to_string()),
            _ => GitBackendError::GitError(e.message().to_string()),
        }
    }
}
