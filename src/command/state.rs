use std::io;

use crate::Config;

pub struct CliState {
    pub config: Config,
    pub stdout: Box<dyn io::Write>,
    pub stderr: Box<dyn io::Write>,
}
