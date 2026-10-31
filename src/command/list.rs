use usage_rs::{Args, RunWith};

use crate::{cache::FileCached, command::state::CliState, error::CliError};

#[derive(Args, Debug)]
pub struct List;

impl RunWith<CliState> for List {
    type Output = crate::result::Result<()>;

    fn run_with(self, mut ctx: CliState) -> Self::Output {
        let templates = FileCached::load(&ctx.config.cache_path).map_err(|_| {
            CliError::Discovery("Cache not found. Run 'git-ignore update' first.".to_string())
        })?;

        for item in &templates {
            writeln!(ctx.stdout, "{item}")?;
        }

        Ok(())
    }
}
