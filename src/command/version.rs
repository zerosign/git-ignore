use usage_rs::{Args, RunWith};

use crate::command::state::CliState;

#[derive(Args, Debug)]
pub struct Version;

impl RunWith<CliState> for Version {
    type Output = crate::result::Result<()>;

    fn run_with(self, mut ctx: CliState) -> Self::Output {

        writeln!(
            ctx.stdout,
            "git-ignore {} (commit: {}, built: {})",
            env!("CARGO_PKG_VERSION"),
            option_env!("VERGEN_GIT_SHA").unwrap_or("unknown"),
            option_env!("VERGEN_BUILD_TIMESTAMP").unwrap_or("unknown"),
        )?;

        Ok(())
    }
}
