use std::{collections::{HashMap, HashSet}, fs, io, path::Path};

use usage_rs::{Args, RunWith};

use crate::{
    Config, TemplateSource, error::{CliError, GenerateError, SyncError}, patch::Patcher, tables::{METADATA_TABLE, TEMPLATES_TABLE},
};

#[derive(Args, Debug)]
pub struct Patch {
    #[usage(short, long)]
    pub(crate) dry_run: bool,
    pub(crate) templates: Vec<String>,
}

/// Populates and resolves the requested templates from active sources.
///
/// # Errors
/// Returns an error if the database cannot be read or template retrieval fails.
pub fn collect_templates<D: redb::ReadableDatabase, S: std::hash::BuildHasher>(
    names: HashSet<String, S>,
    sources: &[TemplateSource],
    db: &D,
) -> Result<HashMap<String, String>, GenerateError> {
    let mut contents = HashMap::new();

    let txn = db.begin_read()?;

    let templates = txn.open_table(TEMPLATES_TABLE)?;
    let versions = txn.open_table(METADATA_TABLE)?;

    // Filter sources to only include those that have been successfully synced
    let sources: Vec<_> = sources
        .iter()
        .filter(|s| {
            let key = s.version_key();
            versions.get(key.as_str()).is_ok_and(|v| v.is_some())
        })
        .collect();

    for name in names {
        // 1. Map the template to the potential database keys we need to check
        let keys = match name.split_once('/') {
            Some((source, name)) => vec![TemplateSource::key_of(source, name)],
            None => sources
                .iter()
                .map(|s| TemplateSource::key_of(&s.name, &name))
                .collect(),
        };

        // 2. Fetch and aggregate all matching template contents
        let content: String = keys
            .into_iter()
            .filter_map(|key| templates.get(key.as_str()).ok().flatten())
            .map(|guard| guard.value().trim().to_string()) // Trim individual parts
            .collect::<Vec<_>>()
            .join("\n\n"); // Use double newline for clean separation between sources

        // 3. Insert if found, otherwise warn
        if content.is_empty() {
            eprintln!("Warning: Template '{name}' not found in any active source");
        } else {
            contents.insert(name.clone(), content);
        }
    }

    Ok(contents)
}



/// Generates the .gitignore output by assembling and optional patching of templates.
///
/// # Errors
/// Returns `GenerateError` if template lookup, writing, or patch execution fails.
pub fn generate_template<
    D: redb::ReadableDatabase,
    W: io::Write,
    P: AsRef<Path>,
    S: std::hash::BuildHasher,
>(
    db: &D,
    names: HashSet<String, S>,
    sources: &[TemplateSource],
    target: P,
    patch: bool,
    mut writer: W,
) -> Result<(), GenerateError> {
    let templates = collect_templates(names, sources, db)?;

    if templates.is_empty() {
        return Err(GenerateError::Discovery(
            "No matching templates found.".to_string(),
        ));
    }

    let fpath = target.as_ref().join(".gitignore");

    // read the content for the gitignore
    let content = fpath
        .exists()
        .then(|| fs::read_to_string(&fpath).ok())
        .flatten()
        .unwrap_or_default();

    match (Patcher::parse(&content).patch(templates), patch) {
        (Some(blocks), true) => {
            // TODO: this need to be better
            fs::write(&fpath, format!("{}", blocks))?;
            writeln!(writer, "Updated .gitignore")?;
        }
        (Some(blocks), false) => {
            writeln!(writer, "{}", blocks)?;
        }
        _ => {
            writeln!(writer, ".gitignore is already up to date")?;
        }
    }

    Ok(())
}

impl RunWith<Config> for Patch {
    type Output = crate::result::Result<()>;

    fn run_with(self, ctx: Config) -> Self::Output {
        if !ctx.db_path.exists() {
            return Err(CliError::Discovery(
                "Database not found. Run with -u first.".to_string(),
            ));
        }

        // TODO: need to resolve this without cloning i guess
        let templates = self.templates.clone();

        let templates: HashSet<String> = HashSet::from_iter(templates.into_iter());

        let db = redb::Builder::new()
            .open_read_only(&ctx.db_path)
            .map_err(SyncError::from)?;

        let stdout = std::io::stdout();

        generate_template(
            &db,
            templates,
            &ctx.sources,
            &ctx.project_path,
            self.dry_run,
            stdout.lock(),
        )?;

        Ok(())
    }
}
