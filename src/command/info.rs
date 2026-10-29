use std::io::{self, Write};

use redb::ReadableTableMetadata;
use usage_rs::{Args, RunWith};

use crate::{Config, error::{InfoError, SyncError}, tables::TEMPLATES_TABLE};

#[derive(Args, Debug)]
pub struct Info;

/// Displays database and system integrity info to the writer.
///
/// # Errors
/// Returns `InfoError` if database querying or writing to the output buffer fails.
pub fn show_info<D: redb::ReadableDatabase, W: Write>(
    config: &Config,
    db: &D,
    writer: W,
) -> Result<(), InfoError> {
    let mut buffered = std::io::BufWriter::new(writer);

    for source in &config.sources {
        writeln!(buffered, "Source [{}]:", source.name)?;
        writeln!(buffered, "  URL:  {}", source.url)?;
        writeln!(buffered, "  Path: {}", source.path.display())?;

        if let Ok(repo) = gix::open(&source.path)
            && let Ok(head) = repo.head()
        {
            writeln!(
                buffered,
                "  HEAD: {}",
                head.id()
                    .map_or_else(|| "Unknown".to_string(), |id| id.to_string())
            )?;
        }
    }

    writeln!(buffered, "Database Path:\t{}", config.db_path.display())?;
    writeln!(buffered, "FST Index Path:\t{}", config.fst_path.display())?;

    {
        let checksum = config.checksum();

        if let Some(hashed) = checksum.0 {
            writeln!(buffered, "Database Hash:\t{hashed} (SHA256)")?;
        }

        if let Some(hashed) = checksum.1 {
            writeln!(buffered, "FST Index Hash:\t{hashed} (SHA256)")?;
        }
    }

    let read_txn = db.begin_read()?;

    if let Ok(templates_table) = read_txn.open_table(TEMPLATES_TABLE) {
        writeln!(
            buffered,
            "Template Count:\t{}",
            templates_table.len().unwrap_or(0)
        )?;
    }

    buffered.flush()?;

    Ok(())
}

impl RunWith<Config> for Info {
    type Output = crate::result::Result<()>;

    fn run_with(self, ctx: Config) -> Self::Output {
        // Handle CTRL+C gracefully for gix operations.
        // Setting to 2 interrupts will cause an immediate hard exit on the second press.
        // The first interrupt allows gix to try and stop ongoing network/file operations cleanly.
        unsafe {
            let _ = gix::interrupt::init_handler(2, || {});
        }

        let db = redb::Builder::new()
            .open_read_only(&ctx.db_path)
            .map_err(SyncError::from)?;

        let stdout = io::stdout();

        show_info(&ctx, &db, stdout.lock())?;

        Ok(())
    }
}
