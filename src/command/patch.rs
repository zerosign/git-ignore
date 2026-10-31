use std::{collections::HashSet, fs};

use usage_rs::{Args, RunWith};

use crate::{command::state::CliState, error::CliError, patch::Patcher, storage::TemplateStorage};

#[derive(Args, Debug)]
pub struct Patch {
    #[usage(short, long)]
    pub(crate) dry_run: bool,
    pub(crate) templates: Vec<String>,
}

impl RunWith<CliState> for Patch {
    type Output = crate::result::Result<()>;

    fn run_with(self, mut ctx: CliState) -> Self::Output {
        if !ctx.config.db_path.exists() {
            return Err(CliError::Discovery(
                "Database not found. Run with -u first.".to_string(),
            ));
        }

        if self.templates.is_empty() {
            writeln!(&mut ctx.stdout, "templates is empty.")?;
            return Ok(());
        }

        let storage = TemplateStorage::open(ctx.config.db_path, ctx.config.cache_path)?;

        let names: HashSet<String> = HashSet::from_iter(self.templates);
        let templates = storage.templates_for(names, &ctx.config.sources)?;

        let fpath = ctx.config.project_path.join(".gitignore");

        // read the content for the gitignore
        let content = fs::read_to_string(&fpath).ok().unwrap_or_default();

        match (Patcher::parse(&content).patch(templates), !self.dry_run) {
            (Some(blocks), true) => {
                fs::write(&fpath, format!("{blocks}"))?;
                writeln!(&mut ctx.stdout, "Updated .gitignore")?;
            }
            (Some(blocks), false) => {
                writeln!(&mut ctx.stdout, "{blocks}")?;
            }
            _ => {
                writeln!(&mut ctx.stdout, ".gitignore is already up to date")?;
            }
        }

        Ok(())
    }
}
