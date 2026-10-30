use usage_rs::{Args, RunWith};

use crate::{command::state::CliState, error::CompactError};

#[derive(Args, Debug)]
pub struct Compact;

impl RunWith<CliState> for Compact {
    type Output = crate::result::Result<()>;

    fn run_with(self, mut ctx: CliState) -> Self::Output {
        let mut db = redb::Database::open(ctx.config.db_path).map_err(CompactError::from)?;
        db.compact().map_err(CompactError::from)?;

        writeln!(ctx.stdout.as_mut(), "Database compacted successfully.")?;

        Ok(())
    }
}
