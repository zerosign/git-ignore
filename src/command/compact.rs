use usage_rs::{Args, RunWith};

use crate::{Config, error::CompactError};

#[derive(Args, Debug)]
pub struct Compact;

impl RunWith<Config> for Compact {
    type Output = crate::result::Result<()>;

    fn run_with(self, ctx: Config) -> Self::Output {
        let mut db = redb::Database::open(ctx.db_path).map_err(CompactError::from)?;

        db.compact().map_err(CompactError::from)?;

        println!("Database compacted successfully.");

        Ok(())
    }
}
