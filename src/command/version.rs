use std::io::{self, Write};

use usage_rs::{Args, RunWith};

use crate::Config;

#[derive(Args, Debug)]
pub struct Version;

impl RunWith<Config> for Version {
    type Output = crate::result::Result<()>;

    fn run_with(self, _ctx: Config) -> Self::Output {
        let stdout = io::stdout();

        writeln!(
            stdout.lock(),
            "git-ignore {} (commit: {}, built: {})",
            env!("CARGO_PKG_VERSION"),
            option_env!("VERGEN_GIT_SHA").unwrap_or("unknown"),
            option_env!("VERGEN_BUILD_TIMESTAMP").unwrap_or("unknown"),
        )?;

        Ok(())
    }
}
