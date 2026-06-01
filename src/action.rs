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
        templates_to_fetch: HashSet<String>,
        patch: bool,
    },
}

impl CliAction {
    pub fn from_args(args: &GitIgnoreArgs, _config: &Config) -> Result<Self> {
        if args.version {
            return Ok(Self::ShowVersion);
        }

        if args.info {
            return Ok(Self::ShowInfo);
        }

        if args.list {
            return Ok(Self::ListTemplates);
        }

        if args.compact {
            return Ok(Self::Compact);
        }

        if args.update {
            return Ok(Self::UpdateOnly);
        }

        let mut templates_to_fetch = HashSet::new();

        for t_arg in &args.templates {
            for t in t_arg.split(',') {
                let trimmed = t.trim();
                if !trimmed.is_empty() {
                    templates_to_fetch.insert(trimmed.to_string());
                }
            }
        }

        if templates_to_fetch.is_empty() {
            Err(CliError::Discovery("No templates specified.".to_string()))
        } else {
            Ok(Self::Generate {
                templates_to_fetch,
                patch: args.patch,
            })
        }
    }
}

pub fn populate_templates<D: redb::ReadableDatabase>(
    templates_to_fetch: &HashSet<String>,
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


    // TODO(@zerosign): refactor this by spliting the codes later on
    for template in templates_to_fetch {
        let mut content = String::new();
        let mut found = false;

        if let Some((source_name, template_name)) = template.split_once('/') {
            let key = format!("{}/{}", source_name, template_name.to_lowercase());

            if let Ok(Some(value)) = table.get(key.as_str()) {
                content.push_str(value.value());
                found = true;
            }
        } else {
            let template_lower = template.to_lowercase();

            for source in active_sources.iter() {
                let key = format!("{}/{}", source.name, template_lower);
                if let Ok(Some(value)) = table.get(key.as_str()) {
                    if !content.is_empty() && !content.ends_with('\n') {
                        content.push('\n');
                    }
                    content.push_str(value.value());
                    found = true;
                }
            }
        }

        if found {
            contents.insert(template.clone(), content);
        } else {
            eprintln!(
                "Warning: Template '{}' not found in any active source",
                template
            );
        }
    }

    Ok(contents)
}

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
        writeln!(buffered, "{}", name)?;
    }

    buffered.flush()?;
    Ok(())
}

pub fn generate_template<D: redb::ReadableDatabase, W: io::Write, P: AsRef<Path>>(
    db: &D,
    templates: &HashSet<String>,
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

    if patch {
        let content = if gitignore_fpath.exists() {
            fs::read_to_string(&gitignore_fpath)?
        } else {
            String::new()
        };

        let mut patcher = Patcher::parse(&content);
        patcher.patch(templates_map);
        let new_content = patcher.serialize();

        if new_content != content {
            fs::write(&gitignore_fpath, new_content)?;
            writeln!(writer, "Updated .gitignore")?;
        } else {
            writeln!(writer, ".gitignore is already up to date")?;
        }
    } else {
        let mut patcher = Patcher::parse("");
        patcher.patch(templates_map);
        let content = patcher.serialize();

        write!(writer, "{}", content.trim())?;
        writeln!(writer)?;
    }

    Ok(())
}

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
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "Unknown".to_string())
            )?;
        }
    }

    writeln!(buffered, "Database Path:   {}", config.db_path.display())?;
    writeln!(buffered, "FST Index Path:  {}", config.fst_path.display())?;

    if let Ok(db_bytes) = fs::read(&config.db_path) {
        writeln!(
            buffered,
            "Database Hash:   {:x} (SHA256)",
            Sha256::digest(&db_bytes)
        )?;
    }

    if let Ok(fst_bytes) = fs::read(&config.fst_path) {
        writeln!(
            buffered,
            "FST Index Hash:  {:x} (SHA256)",
            Sha256::digest(&fst_bytes)
        )?;
        writeln!(buffered, "FST Index Size:  {} bytes", fst_bytes.len())?;
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
