use crate::error::CliError;

pub type Result<T, E = CliError> = std::result::Result<T, E>;
