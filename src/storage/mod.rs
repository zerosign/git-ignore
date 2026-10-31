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

    #[must_use]
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use gix::ObjectId;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;
    use crate::sync::{Changed, Changes};

    #[fixture]
    fn storage() -> (TemplateStorage, TempDir) {
        let temp = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = temp.path().join("nested/dir/templates.redb");
        let cache_path = temp.path().join("nested/dir/templates.rkyv");
        let s = TemplateStorage::open(&db_path, &cache_path).expect("open storage");

        (s, temp)
    }

    #[rstest]
    fn test_storage_open_and_lazy_tables(storage: (TemplateStorage, TempDir)) {
        let (s, temp) = storage;

        // Auto-directory creation check
        assert!(temp.path().join("nested/dir/templates.redb").exists());

        // Lazy table creation check (TEMPLATES_TABLE created on demand)
        let count = s.counts().expect("counts on fresh db");

        assert_eq!(count, 0);

        // Lazy table creation check (VERSIONS_TABLE created on demand)
        let version = s.last_version_for("last_commit_hash:default");

        assert_eq!(version, None);
    }

    #[rstest]
    fn test_storage_save_changes_and_retrieval(storage: (TemplateStorage, TempDir)) {
        let (s, _temp) = storage;

        let source = TemplateSource {
            name: "default".to_string(),
            url: "https://github.com/github/gitignore".to_string(),
            path: PathBuf::from("/tmp/sources/default"),
            branch: None,
        };

        let dummy_oid = ObjectId::null(gix::hash::Kind::Sha1);

        let changes = Changes {
            upserts: vec![
                Changed {
                    namespace: "default/rust".to_string(),
                    oid: dummy_oid,
                    content: "target/\n**/*.rs.bk\n".to_string(),
                },
                Changed {
                    namespace: "default/python".to_string(),
                    oid: dummy_oid,
                    content: "__pycache__/\n*.pyc\n".to_string(),
                },
            ],
            deletes: vec![],
        };

        // 1. Save changes and version
        let vkey = source.version_key();
        s.save_changes(&vkey, "commit_sha_123", changes)
            .expect("save_changes should succeed");

        // 2. Verify version hash is stored and retrievable
        assert_eq!(s.last_version_for(&vkey).as_deref(), Some("commit_sha_123"));

        // 3. Verify counts
        assert_eq!(s.counts().expect("counts"), 2);

        // 4. Verify templates_for with unqualified name ("rust")
        let queried = s
            .templates_for(
                HashSet::from(["rust".to_string()]),
                std::slice::from_ref(&source),
            )
            .expect("templates_for");

        assert_eq!(
            queried.get("rust").map(String::as_str),
            Some("target/\n**/*.rs.bk")
        );

        // 5. Verify templates_for with qualified name ("default/python")
        let queried = s
            .templates_for(
                HashSet::from(["default/python".to_string()]),
                std::slice::from_ref(&source),
            )
            .expect("templates_for");

        assert_eq!(
            queried.get("default/python").map(String::as_str),
            Some("__pycache__/\n*.pyc")
        );

        // 6. Non-existent template should not be returned
        let queried = s
            .templates_for(
                HashSet::from(["nonexistent".to_string()]),
                std::slice::from_ref(&source),
            )
            .expect("templates_for");

        assert!(queried.is_empty());
    }

    #[rstest]
    fn test_storage_save_changes_deletes(storage: (TemplateStorage, TempDir)) {
        let (s, _temp) = storage;

        let source = TemplateSource {
            name: "default".to_string(),
            url: "https://github.com/github/gitignore".to_string(),
            path: PathBuf::from("/tmp/sources/default"),
            branch: None,
        };

        let dummy_oid = ObjectId::null(gix::hash::Kind::Sha1);
        let vkey = source.version_key();

        // 1. Initial insert
        s.save_changes(
            &vkey,
            "v1",
            Changes {
                upserts: vec![Changed {
                    namespace: "default/rust".to_string(),
                    oid: dummy_oid,
                    content: "target/\n".to_string(),
                }],
                deletes: vec![],
            },
        )
        .expect("insert");

        assert_eq!(s.counts().expect("counts"), 1);

        // 2. Perform delete
        s.save_changes(
            &vkey,
            "v2",
            Changes {
                upserts: vec![],
                deletes: vec!["default/rust".to_string()],
            },
        )
        .expect("delete");

        // 3. Verify counts decreases and retrieval returns empty
        assert_eq!(s.counts().expect("counts"), 0);
        
        let queried = s
            .templates_for(
                HashSet::from(["rust".to_string()]),
                std::slice::from_ref(&source),
            )
            .expect("templates_for");

        assert!(queried.is_empty());
    }

    #[rstest]
    fn test_storage_build_cache_and_load(storage: (TemplateStorage, TempDir)) {
        let (s, temp) = storage;

        let dummy_oid = ObjectId::null(gix::hash::Kind::Sha1);
        let changes = Changes {
            upserts: vec![
                Changed {
                    namespace: "default/rust".to_string(),
                    oid: dummy_oid,
                    content: "target/\n".to_string(),
                },
                Changed {
                    namespace: "default/go".to_string(),
                    oid: dummy_oid,
                    content: "*.exe\n".to_string(),
                },
                Changed {
                    namespace: "custom/python".to_string(),
                    oid: dummy_oid,
                    content: "*.pyc\n".to_string(),
                },
            ],
            deletes: vec![],
        };

        s.save_changes("last_commit_hash:default", "sha1", changes)
            .expect("save_changes");

        // 1. Build binary rkyv cache
        s.build_cache().expect("build_cache");

        let cache_file = temp.path().join("nested/dir/templates.rkyv");

        assert!(cache_file.exists());

        // 2. Load cache with FileCached and inspect names
        let templates = FileCached::load(&cache_file).expect("FileCached::load");
        let names: Vec<&str> = (&templates).into_iter().collect();

        // Names should be namespace-stripped, sorted, and deduplicated
        assert_eq!(names, vec!["go", "python", "rust"]);
    }

    #[rstest]
    fn test_storage_compact_and_checksum(mut storage: (TemplateStorage, TempDir)) {
        let (s, _temp) = &mut storage;

        // 1. Before cache is built, db hash exists, cache hash is None
        let (db_hash, cache_hash) = s.checksum();

        assert!(db_hash.is_some());

        let db_hash = db_hash.unwrap();

        assert_eq!(db_hash.len(), 64);
        assert!(db_hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(cache_hash, None);

        // 2. Build cache and verify both hashes exist
        // Note: list_names inside build_cache initializes TEMPLATES_TABLE on demand via table_ro,
        // which commits table metadata to redb.
        s.build_cache().expect("build_cache");

        let (db_hash_after, cache_hash_after) = s.checksum();

        assert!(db_hash_after.is_some());
        assert!(cache_hash_after.is_some());

        let cache_hash = cache_hash_after.unwrap();

        assert_eq!(cache_hash.len(), 64);
        assert!(cache_hash.chars().all(|c| c.is_ascii_hexdigit()));

        // 3. Compact should complete successfully and db hash remains valid
        s.compact().expect("compact should succeed");

        let (db_hash_final, _) = s.checksum();

        assert!(db_hash_final.is_some());
    }
}
