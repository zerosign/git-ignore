use thiserror::Error;

pub use crate::config::ConfigError;
pub use crate::storage::StorageError;
pub use crate::sync::SyncError;

#[derive(Error, Debug)]
pub enum CliError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Sync(#[from] SyncError),
    #[error("Discovery error: {0}")]
    Discovery(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
