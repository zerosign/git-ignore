use usage_rs::{Args, RunWith};

use crate::{command::state::CliState, storage::TemplateStorage};

#[derive(Args, Debug)]
pub struct Compact;

impl RunWith<CliState> for Compact {
    type Output = crate::result::Result<()>;

    fn run_with(self, mut ctx: CliState) -> Self::Output {
        let mut storage = TemplateStorage::open(&ctx.config.db_path, &ctx.config.cache_path)
            .expect("can't open template storage");

        storage.compact().expect("fails to do compactness");

        writeln!(ctx.stdout.as_mut(), "Database compacted successfully.")?;

        Ok(())
    }
}
