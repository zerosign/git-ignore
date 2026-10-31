use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

mod cache;
mod tables;

pub use cache::{FileCached, Templates};
use redb::{
    Database, ReadTransaction, ReadableDatabase, ReadableTable, ReadableTableMetadata, StorageError,
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

        let db = Database::open(&db_path)
            .or_else(|_| {
                let _ = db_path.parent().map(std::fs::create_dir_all);
                Database::create(&db_path)
            })
            .expect("fails to open template db");

        Ok(Self {
            db,
            db_path,
            cache_path,
        })
    }
}

impl TemplateStorage {
    pub fn build_cache(&self) -> Result<(), StorageError> {
        FileCached::save(&self.cache_path, self.list_names())
            .expect("fails to save template names cache");

        Ok(())
    }

    pub fn compact(&mut self) -> Result<(), StorageError> {
        self.db.compact().expect("error when compacting storage");
        Ok(())
    }
}
// 1 read txn
// - Storage::versions_for => list versions for list of sources
// - Storage::contents_for => list contents for list of source versions
//
// Storage::templates_for => list template contents based on source (based on above helper function with shared read txn)

impl TemplateStorage {
    fn list_names(&self) -> Vec<String> {
        let rtx = self.db.begin_read().expect("fails to read only");

        // this contains source/template => template content
        let tmpls = match rtx.open_table(TEMPLATES_TABLE) {
            Ok(table) => table,
            _ => {
                {
                    let wtx = self.db.begin_write().expect("can't create write tx");

                    wtx.open_table(TEMPLATES_TABLE).expect("can't create table");

                    wtx.commit().expect("can't commit");
                }

                rtx.open_table(TEMPLATES_TABLE)
                    .expect("can't open table still")
            }
        };

        let mut names: Vec<String> = tmpls
            .iter()
            .expect("fails to scan keys from template table")
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

        names
    }

    // TODO: this won't do hmm instead we should provide a filter function that do check given
    // version key I guess
    fn sources_for<'a>(
        &self,
        rtx: &ReadTransaction,
        sources: &'a [TemplateSource],
    ) -> Vec<&'a TemplateSource> {
        let versions = match rtx.open_table(VERSIONS_TABLE) {
            Ok(table) => table,
            _ => {
                {
                    self.db
                        .begin_write()
                        .and_then(|wtx| {
                            wtx.open_table(VERSIONS_TABLE).expect("can't create table");
                            wtx.commit().expect("can't commit");
                            Ok(())
                        })
                        .expect("can't create table");
                }

                rtx.open_table(VERSIONS_TABLE)
                    .expect("can't open table still")
            }
        };

        sources
            .iter()
            .filter(|s| {
                let key = s.version_key();
                versions.get(key.as_str()).is_ok_and(|v| v.is_some())
            })
            .collect()
    }

    fn contents_for<S: std::hash::BuildHasher>(
        &self,
        rtx: &ReadTransaction,
        names: HashSet<String, S>,
        sources: &[&TemplateSource],
    ) -> Result<HashMap<String, String>, StorageError> {
        let mut contents = HashMap::new();

        // this contains source/template => template content
        let tmpls = match rtx.open_table(TEMPLATES_TABLE) {
            Ok(table) => table,
            _ => {
                {
                    self.db
                        .begin_write()
                        .and_then(|wtx| {
                            wtx.open_table(TEMPLATES_TABLE).expect("can't create table");
                            wtx.commit().expect("can't commit");
                            Ok(())
                        })
                        .expect("can't create table");
                }

                rtx.open_table(TEMPLATES_TABLE)
                    .expect("can't open table still")
            }
        };

        for name in names {
            // 1. Map the template to the potential database keys we need to check
            let vkeys = if let Some((source, tname)) = name.split_once('/') {
                vec![TemplateSource::key_of(source, &tname.to_lowercase())]
            } else {
                let lower = name.to_lowercase();
                sources
                    .iter()
                    .map(|s| TemplateSource::key_of(&s.name, &lower))
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
    // TODO
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
        let rtx = self.db.begin_read().expect("can't open read tx for db");

        let size = match rtx.open_table(TEMPLATES_TABLE) {
            Ok(table) => table,
            _ => {
                self.db
                    .begin_write()
                    .and_then(|wtx| {
                        wtx.open_table(TEMPLATES_TABLE).expect("can't create table");
                        wtx.commit().expect("can't commit");
                        Ok(())
                    })
                    .expect("can't create table");

                rtx.open_table(TEMPLATES_TABLE).expect("can't open table")
            }
        }
        .len()
        .unwrap_or_default() as usize;

        Ok(size)
    }

    #[must_use]
    pub fn last_version_for(&self, vkey: &str) -> Option<String> {
        // old_oid <- Storage::last_version_of(vkey)
        //
        let rtx = self.db.begin_read().expect("fails to read only");

        let versions = match rtx.open_table(VERSIONS_TABLE) {
            Ok(table) => table,
            _ => {
                {
                    self.db
                        .begin_write()
                        .and_then(|wtx| {
                            wtx.open_table(VERSIONS_TABLE).expect("can't create table");
                            wtx.commit().expect("can't commit");
                            Ok(())
                        })
                        .expect("can't create table");
                }

                rtx.open_table(VERSIONS_TABLE)
                    .expect("can't open table still")
            }
        };

        versions
            .get(vkey)
            .ok()
            .flatten()
            .and_then(|v| str::from_utf8(v.value()).ok().map(str::to_string))
    }

    pub fn templates_for<S: std::hash::BuildHasher>(
        &self,
        names: HashSet<String, S>,
        sources: &[TemplateSource],
    ) -> Result<HashMap<String, String>, StorageError> {
        let txn = self.db.begin_read().expect("can't create read tx for db");

        // 1 read txn
        // - Storage::versions_for => list versions for list of sources
        // - Storage::contents_for => list contents for list of source versions
        //
        // Storage::templates_for => list template contents based on source (based on above helper function with shared read txn)
        //
        // Filter sources to only include those that have been successfully synced
        let sources = self.sources_for(&txn, sources);

        self.contents_for(&txn, names, &sources)
    }

    pub fn save_changes(&self, vkey: &str, version: &str, changes: Changes) {
        let wtx = self.db.begin_write().expect("fails to do write tx");

        {
            // this contains source/template => template content
            let mut tmpls = wtx
                .open_table(TEMPLATES_TABLE)
                .expect("fails to open template table for write");

            // this contains source/template => template object id
            let mut toids = wtx
                .open_table(TEMPLATES_OIDS_TABLE)
                .expect("fails to template oids for write");

            // delete all keys that need to be deleted first
            for item in changes.deletes {
                tmpls
                    .remove(item.as_str())
                    .expect("fails to delete tmpl content");

                toids
                    .remove(item.as_str())
                    .expect("fails to delete tmpl oids");
            }

            // upserts all keys that need to updated
            for item in changes.upserts {
                // insert template values first
                //
                tmpls
                    .insert(item.namespace.as_str(), item.content.as_str())
                    .expect("fails to insert template content");

                // insert template oids
                toids
                    .insert(item.namespace.as_str(), item.oid.to_string().as_str())
                    .expect("fails to insert template oids");
            }

            let mut versions = wtx
                .open_table(VERSIONS_TABLE)
                .expect("fails to open versions table for writes");

            versions
                .insert(vkey, version.as_bytes())
                .expect("fails to update version of source");

            // TODO: or just drop both tmpls, toids & versions in here hmm
            // drop(tmpls);
            // drop(toids);
            // drop(versions);
        }

        wtx.commit().expect("fails to delete");
    }
}
