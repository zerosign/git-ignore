use usage_rs::{Args, RunWith};

use crate::command::state::CliState;

#[derive(Args, Debug)]
pub struct Update {
    #[usage(short, long)]
    pub(crate) force: bool,
}

impl RunWith<CliState> for Update {
    type Output = crate::result::Result<()>;

    fn run_with(self, ctx: CliState) -> Self::Output {
        // Handle CTRL+C gracefully for gix operations.
        // Setting to 2 interrupts will cause an immediate hard exit on the second press.
        // The first interrupt allows gix to try and stop ongoing network/file operations cleanly.
        unsafe {
            let _ = gix::interrupt::init_handler(2, || {});
        }

        use crate::sync_ as sync;

        sync::sync(&ctx.config, self.force)?;

        Ok(())
    }
}
