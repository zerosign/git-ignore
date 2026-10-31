use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

mod cache;
mod error;
mod tables;

pub use cache::{FileCached, Templates};
pub use error::StorageError;
use redb::{
    Database, ReadOnlyTable, ReadTransaction, ReadableDatabase, ReadableTable,
    ReadableTableMetadata, TableDefinition,
};
use sha2::{Digest, Sha256};
use tables::{TEMPLATES_OIDS_TABLE, TEMPLATES_TABLE, VERSIONS_TABLE};

use crate::{TemplateSource, sync::Changes};

pub struct TemplateStorage {
    db: redb::Database,
    db_path: PathBuf,
    cache_path: PathBuf,
}

impl TemplateStorage {
    pub fn open<P>(db_path: P, cache_path: P) -> Result<Self, StorageError>
    where
        P: AsRef<Path>,
    {
        let db_path = PathBuf::from(db_path.as_ref());
        let cache_path = PathBuf::from(cache_path.as_ref());

        let db = Database::open(&db_path).or_else(|_| {
            if let Some(parent) = db_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            Database::create(&db_path).map_err(StorageError::from)
        })?;

        Ok(Self {
            db,
            db_path,
            cache_path,
        })
    }

    pub fn build_cache(&self) -> Result<(), StorageError> {
        FileCached::save(&self.cache_path, self.list_names()?)?;
        Ok(())
    }

    pub fn compact(&mut self) -> Result<(), StorageError> {
        self.db.compact()?;
        Ok(())
    }
}

impl TemplateStorage {
    fn table_ro<K: redb::Key + 'static, V: redb::Value + 'static>(
        &self,
        rtx: &ReadTransaction,
        table: TableDefinition<K, V>,
    ) -> Result<ReadOnlyTable<K, V>, StorageError> {
        match rtx.open_table(table) {
            Ok(table) => Ok(table),
            Err(redb::TableError::TableDoesNotExist(_)) => {
                let wtx = self.db.begin_write()?;
                wtx.open_table(table)?;
                wtx.commit()?;

                let new_rtx = self.db.begin_read()?;
                Ok(new_rtx.open_table(table)?)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn list_names(&self) -> Result<Vec<String>, StorageError> {
        let rtx = self.db.begin_read()?;
        let tmpls = self.table_ro(&rtx, TEMPLATES_TABLE)?;

        let mut names: Vec<String> = tmpls
            .iter()?
            .flatten()
            .map(|(k, _)| {
                let val = k.value();
                val.split_once('/')
                    .map_or(val, |(_, name)| name)
                    .to_string()
            })
            .collect();

        names.sort_unstable();
        names.dedup();

        Ok(names)
    }

    fn sources_for<'a>(
        &self,
        rtx: &ReadTransaction,
        sources: &'a [TemplateSource],
    ) -> Result<Vec<&'a TemplateSource>, StorageError> {
        let versions = self.table_ro(rtx, VERSIONS_TABLE)?;

        let sources = sources
            .iter()
            .filter(|s| {
                let key = s.version_key();
                versions.get(key.as_str()).is_ok_and(|v| v.is_some())
            })
            .collect();

        Ok(sources)
    }

    fn contents_for<S: std::hash::BuildHasher>(
        &self,
        rtx: &ReadTransaction,
        names: HashSet<String, S>,
        sources: &[&TemplateSource],
    ) -> Result<HashMap<String, String>, StorageError> {
        let mut contents = HashMap::new();
        let tmpls = self.table_ro(rtx, TEMPLATES_TABLE)?;

        for name in names {
            // 1. Map the template to the potential database keys we need to check
            let vkeys = if let Some((source, tname)) = name.split_once('/') {
                vec![TemplateSource::key_of(source, &tname.to_lowercase())]
            } else {
                sources
                    .iter()
                    // source, name
                    .map(|s| TemplateSource::key_of(&s.name, &name.to_lowercase()))
                    .collect()
            };

            // 2. Fetch and aggregate all matching template contents
            let content: String = vkeys
                .into_iter()
                .filter_map(|vkey| tmpls.get(vkey.as_str()).ok().flatten())
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

impl TemplateStorage {
    #[must_use]
    pub fn checksum(&self) -> (Option<String>, Option<String>) {
        (
            format_file_hash(&self.db_path),
            format_file_hash(&self.cache_path),
        )
    }

    pub fn counts(&self) -> Result<usize, StorageError> {
        let rtx = self.db.begin_read()?;
        let tmpls = self.table_ro(&rtx, TEMPLATES_TABLE)?;
        let size = tmpls.len().unwrap_or_default() as usize;
        Ok(size)
    }

    pub fn last_version_for(&self, vkey: &str) -> Option<String> {
        let rtx = self.db.begin_read().ok()?;
        let versions = self.table_ro(&rtx, VERSIONS_TABLE).ok()?;

        versions
            .get(vkey)
            .ok()
            .flatten()
            .and_then(|v| std::str::from_utf8(v.value()).ok().map(str::to_string))
    }

    pub fn templates_for<S: std::hash::BuildHasher>(
        &self,
        names: HashSet<String, S>,
        sources: &[TemplateSource],
    ) -> Result<HashMap<String, String>, StorageError> {
        let txn = self.db.begin_read()?;
        let sources = self.sources_for(&txn, sources)?;
        self.contents_for(&txn, names, &sources)
    }

    pub fn save_changes(
        &self,
        vkey: &str,
        version: &str,
        changes: Changes,
    ) -> Result<(), StorageError> {
        let wtx = self.db.begin_write()?;

        {
            let mut tmpls = wtx.open_table(TEMPLATES_TABLE)?;
            let mut toids = wtx.open_table(TEMPLATES_OIDS_TABLE)?;

            for item in changes.deletes {
                tmpls.remove(item.as_str())?;
                toids.remove(item.as_str())?;
            }

            for item in changes.upserts {
                tmpls.insert(item.namespace.as_str(), item.content.as_str())?;
                toids.insert(item.namespace.as_str(), item.oid.to_string().as_str())?;
            }

            let mut versions = wtx.open_table(VERSIONS_TABLE)?;
            versions.insert(vkey, version.as_bytes())?;
        }

        wtx.commit()?;
        Ok(())
    }
}
