use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    io::{self, Write},
    path::Path,
};

use fst::{IntoStreamer, Set, Streamer};
use redb::ReadableTableMetadata;
use sha2::{Digest, Sha256};

use crate::{
    args::GitIgnoreArgs,
    config::{Config, TemplateSource},
    defs::TEMPLATES_TABLE,
    error::{CliError, GenerateError, InfoError, ListError},
    patch::Patcher,
    result::Result,
};

#[derive(Debug)]
pub enum CliAction {
    ShowVersion,
    ShowInfo,
    ListTemplates,
    UpdateOnly,
    Compact,
    Generate {
        templates: HashSet<String>,
        patch: bool,
    },
}

impl CliAction {
    /// Resolves the `CliAction` from `GitIgnoreArgs`.
    ///
    /// # Errors
    /// Returns `CliError` if templates are not specified.
    pub fn from_args(args: &GitIgnoreArgs, _config: &Config) -> Result<Self> {
        // I found this is better than if and else (personal opinion)
        match (
            args.version,
            args.info,
            args.list,
            args.compact,
            args.update || args.force,
        ) {
            (true, _, _, _, _) => Ok(Self::ShowVersion),
            (_, true, _, _, _) => Ok(Self::ShowInfo),
            (_, _, true, _, _) => Ok(Self::ListTemplates),
            (_, _, _, true, _) => Ok(Self::Compact),
            _ => {
                let templates: HashSet<String> = args
                    .templates
                    .iter()
                    .flat_map(|t_arg| t_arg.split(','))
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(String::from)
                    .collect();

                match (templates.is_empty(), args.update || args.force) {
                    (true, true) => Ok(Self::UpdateOnly),
                    (true, false) => {
                        Err(CliError::Discovery("No templates specified.".to_string()))
                    }
                    _ => Ok(Self::Generate {
                        templates,
                        patch: args.patch,
                    }),
                }
            }
        }
    }
}

/// Populates and resolves the requested templates from active sources.
///
/// # Errors
/// Returns an error if the database cannot be read or template retrieval fails.
pub fn populate_templates<D: redb::ReadableDatabase, S: std::hash::BuildHasher>(
    templates: &HashSet<String, S>,
    sources: &[TemplateSource],
    db: &D,
) -> Result<HashMap<String, String>, GenerateError> {
    let mut contents = HashMap::new();

    let read_txn = db.begin_read()?;

    let table = read_txn.open_table(TEMPLATES_TABLE)?;
    let meta_table = read_txn.open_table(crate::defs::METADATA_TABLE)?;

    // Filter sources to only include those that have been successfully synced
    let active_sources: Vec<_> = sources
        .iter()
        .filter(|s| {
            let key = format!("last_commit_hash:{}", s.name);
            meta_table.get(key.as_str()).is_ok_and(|v| v.is_some())
        })
        .collect();

    for template in templates {
        // 1. Map the template to the potential database keys we need to check
        let keys = match template.split_once('/') {
            Some((source, name)) => vec![format!("{}/{}", source, name.to_lowercase())],
            None => active_sources
                .iter()
                .map(|s| format!("{}/{}", s.name, template.to_lowercase()))
                .collect(),
        };

        // 2. Fetch and aggregate all matching template contents
        let content: String = keys
            .into_iter()
            .filter_map(|key| table.get(key.as_str()).ok().flatten())
            .map(|guard| guard.value().trim().to_string()) // Trim individual parts
            .collect::<Vec<_>>()
            .join("\n\n"); // Use double newline for clean separation between sources

        // 3. Insert if found, otherwise warn
        if content.is_empty() {
            eprintln!("Warning: Template '{template}' not found in any active source");
        } else {
            contents.insert(template.clone(), content);
        }
    }

    Ok(contents)
}

/// Lists all available templates in the FST index.
///
/// # Errors
/// Returns `ListError` if the FST index is missing, corrupt, or writing to the writer fails.
pub fn list_templates<W: Write>(fst_path: &Path, writer: W) -> Result<(), ListError> {
    if !fst_path.exists() {
        return Err(ListError::Discovery(
            "FST index not found. Run with -u first.".to_string(),
        ));
    }

    let fst_bytes = fs::read(fst_path)?;
    let set = Set::new(fst_bytes).map_err(|e| ListError::Fst(e.to_string()))?;

    let mut stream = set.into_stream();

    let mut unique_names = BTreeSet::new();

    while let Some(name_bytes) = stream.next() {
        let full_name = String::from_utf8_lossy(name_bytes);

        if let Some((_source, name)) = full_name.split_once('/') {
            unique_names.insert(name.to_string());
        } else {
            unique_names.insert(full_name.to_string());
        }
    }

    let mut buffered = std::io::BufWriter::new(writer);

    for name in unique_names {
        writeln!(buffered, "{name}")?;
    }

    buffered.flush()?;

    Ok(())
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
    templates: &HashSet<String, S>,
    sources: &[TemplateSource],
    project_path: P,
    patch: bool,
    mut writer: W,
) -> Result<(), GenerateError> {
    let templates_map = populate_templates(templates, sources, db)?;

    if templates_map.is_empty() {
        return Err(GenerateError::Discovery(
            "No matching templates found.".to_string(),
        ));
    }

    let gitignore_fpath = project_path.as_ref().join(".gitignore");

    let content = gitignore_fpath
        .exists()
        .then(|| fs::read_to_string(&gitignore_fpath).ok())
        .flatten()
        .unwrap_or_default();

    let mut patcher = Patcher::parse(&content);
    patcher.patch(templates_map);

    let new_content = patcher.serialize();

    match patch {
        true if new_content != content => {
            fs::write(&gitignore_fpath, new_content)?;
            writeln!(writer, "Updated .gitignore")?;
        }
        true => {
            writeln!(writer, ".gitignore is already up to date")?;
        }
        _ => {
            write!(writer, "{}", new_content.trim())?;
            writeln!(writer)?;
        }
    }

    Ok(())
}

fn format_file_hash(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let hash = Sha256::digest(&bytes);
    let mut hex = String::with_capacity(64);

    for b in hash {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }

    Some(hex)
}

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

    writeln!(buffered, "Database Path:   {}", config.db_path.display())?;
    writeln!(buffered, "FST Index Path:  {}", config.fst_path.display())?;

    if let Some(hashed) = format_file_hash(&config.db_path) {
        writeln!(buffered, "Database Hash:   {hashed} (SHA256)")?;
    }

    if let Some(hashed) = format_file_hash(&config.fst_path) {
        writeln!(buffered, "FST Index Hash:  {hashed} (SHA256)")?;
    }

    let read_txn = db.begin_read()?;

    if let Ok(templates_table) = read_txn.open_table(TEMPLATES_TABLE) {
        writeln!(
            buffered,
            "Template Count:  {}",
            templates_table.len().unwrap_or(0)
        )?;
    }

    buffered.flush()?;

    Ok(())
}
