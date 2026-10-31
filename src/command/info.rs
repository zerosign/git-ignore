use std::io::Write;

use usage_rs::{Args, RunWith};

use crate::{command::state::CliState, storage::TemplateStorage};

#[derive(Args, Debug)]
pub struct Info;

impl RunWith<CliState> for Info {
    type Output = crate::result::Result<()>;

    fn run_with(self, ctx: CliState) -> Self::Output {
        // Handle CTRL+C gracefully for gix operations.
        // Setting to 2 interrupts will cause an immediate hard exit on the second press.
        // The first interrupt allows gix to try and stop ongoing network/file operations cleanly.
        unsafe {
            let _ = gix::interrupt::init_handler(2, || {});
        }

        let mut writer = ctx.stdout;

        let storage = TemplateStorage::open(&ctx.config.db_path, &ctx.config.cache_path)?;

        for source in &ctx.config.sources {
            writeln!(writer, "Source [{}]:", source.name)?;
            writeln!(writer, "  URL:  {}", source.url)?;
            writeln!(writer, "  Path: {}", source.path.display())?;

            if let Ok(repo) = gix::open(&source.path)
                && let Ok(head) = repo.head()
            {
                writeln!(
                    writer,
                    "  HEAD: {}",
                    head.id()
                        .map_or_else(|| "Unknown".to_string(), |id| id.to_string())
                )?;
            }
        }

        writeln!(writer, "Database Path:\t{}", ctx.config.db_path.display())?;
        writeln!(writer, "Caches Path:\t{}", ctx.config.cache_path.display())?;

        {
            let checksum = storage.checksum();

            if let Some(hashed) = checksum.0 {
                writeln!(writer, "Database Hash:\t{hashed} (SHA256)")?;
            }

            if let Some(hashed) = checksum.1 {
                writeln!(writer, "Caches Index Hash:\t{hashed} (SHA256)")?;
            }
        }

        writeln!(writer, "Template Count:\t{}", storage.counts()?)?;

        writer.flush()?;

        Ok(())
    }
}
