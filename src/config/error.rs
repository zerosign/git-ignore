use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Environment error: {0}")]
    Env(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Config parse error: {0}")]
    Parse(String),
}
