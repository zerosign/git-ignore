use std::fs::{self, remove_file};
use std::path::Path;
use fst::SetBuilder;
use gix::{ObjectId, Repository};
use redb::{Database, ReadableTable};

use crate::{
    TemplateSource,
    config::Config,
    defs::{METADATA_TABLE, TEMPLATES_OIDS_TABLE, TEMPLATES_TABLE},
    error::SyncError,
    result::Result,
    state::{SyncState, RepoState},
};

/// Operations to be executed inside the database transaction.
pub struct SyncDiff {
    pub upserts: Vec<(String, ObjectId, String)>, // (namespaced_name, oid, source_name)
    pub deletes: Vec<String>,                          // (namespaced_name)
}

impl Default for SyncDiff {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncDiff {
    pub fn new() -> Self {
        Self {
            upserts: vec![],
            deletes: vec![],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.upserts.is_empty() && self.deletes.is_empty()
    }
}

/// Responsible for computing the diff for a single TemplateSource.
pub struct SyncUpdater<'a> {
    source: &'a TemplateSource,
    repo: Repository,
}

impl<'a> SyncUpdater<'a> {
    pub fn new(source: &'a TemplateSource) -> Result<Self, SyncError> {
        let repo = gix::open(&source.path)?;
        Ok(Self { source, repo })
    }

    /// Returns the HEAD OID for the repository.
    pub fn head_oid(&self) -> Result<ObjectId, SyncError> {
        Ok(self.repo
            .head()?
            .id()
            .ok_or_else(|| {
                SyncError::Discovery(format!(
                    "HEAD for {} does not point to a commit",
                    self.source.name
                ))
            })?
            .detach())
    }

    /// Computes the diff against the database and appends to the global diff.
    /// Returns the head_oid if an update is needed.
    pub fn compute_diff(
        &self,
        db: &Database,
        fst_path: &Path,
        global_diff: &mut SyncDiff,
    ) -> Result<Option<ObjectId>, SyncError> {
        let head_oid = self.head_oid()?;
        let state = SyncState::determine(db, &self.source.name, &head_oid, fst_path)?;

        match state {
            SyncState::UpToDate => Ok(None),
            SyncState::Incremental(last_oid) => {
                let old_tree = self.repo.find_object(last_oid)?.peel_to_tree()?;
                let new_tree = self.repo.find_object(head_oid)?.peel_to_tree()?;

                old_tree.changes()?.for_each_to_obtain_tree(&new_tree, |change| {
                    use gix::object::tree::diff::Change;
                    let (location, oid_opt) = match change {
                        Change::Addition { location, id, .. } => (location, Some(id.detach())),
                        Change::Modification { location, id, .. } => (location, Some(id.detach())),
                        Change::Deletion { location, .. } => (location, None),
                        Change::Rewrite { location, id, .. } => (location, Some(id.detach())),
                    };
                    if location.ends_with(b".gitignore") {
                        let name = location
                            .strip_suffix(b".gitignore")
                            .ok_or_else(|| SyncError::GitOp("Invalid suffix".to_string()))?;
                        let name_str = String::from_utf8_lossy(name).to_lowercase();
                        let namespaced_name = format!("{}/{}", self.source.name, name_str);
                        if let Some(oid) = oid_opt {
                            global_diff.upserts.push((namespaced_name, oid, self.source.name.clone()));
                        } else {
                            global_diff.deletes.push(namespaced_name);
                        }
                    }
                    Ok::<gix::diff::tree::visit::Action, SyncError>(
                        gix::diff::tree::visit::Action::Continue(()),
                    )
                })?;
                Ok(Some(head_oid))
            }
            _ => {
                let all = SyncState::fetch_all_templates(&self.repo, &head_oid)?;
                for (name, oid) in all {
                    let namespaced_name = format!("{}/{}", self.source.name, name);
                    global_diff.upserts.push((namespaced_name, oid, self.source.name.clone()));
                }
                Ok(Some(head_oid))
            }
        }
    }

    /// Fetches a blob from the repository.
    pub fn fetch_blob(&self, oid: ObjectId) -> Result<String, SyncError> {
        let blob = self.repo.find_object(oid)?.into_blob();
        Ok(String::from_utf8_lossy(&blob.data).to_string())
    }
}

/// Global orchestrator for the synchronization session.
pub struct SyncManager<'a> {
    config: &'a Config,
}

impl<'a> SyncManager<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    /// Ensures all local repositories exist and are updated if requested.
    pub fn ensure_repos(&self, force_update: bool) -> Result<(), SyncError> {
        for source in &self.config.sources {
            if force_update || !source.path.exists() {
                RepoState::ensure(source, force_update)?;
            }
        }
        Ok(())
    }

    /// Executes the full database sync.
    pub fn sync(&self) -> Result<(), SyncError> {
        // 1. Open Database
        let db = match Database::create(&self.config.db_path) {
            Ok(db) => db,
            Err(_) => {
                let _ = remove_file(&self.config.db_path);
                Database::create(&self.config.db_path)?
            }
        };

        // 2. Compute Diffs
        let mut global_diff = SyncDiff::new();
        let mut updaters = Vec::new();
        let mut updated_sources = Vec::new();

        for source in &self.config.sources {
            let updater = SyncUpdater::new(source)?;
            if let Some(head_oid) = updater.compute_diff(&db, &self.config.fst_path, &mut global_diff)? {
                updated_sources.push((source.name.clone(), head_oid));
            }
            updaters.push(updater);
        }

        if global_diff.is_empty() && updated_sources.is_empty() && self.config.fst_path.exists() {
            return Ok(());
        }

        // 3. Apply Diff in a single transaction
        self.apply_updates(&db, global_diff, &updaters, updated_sources)
    }

    /// Internal helper to apply computed diffs and update metadata/index.
    fn apply_updates(
        &self,
        db: &Database,
        global_diff: SyncDiff,
        updaters: &[SyncUpdater],
        updated_sources: Vec<(String, ObjectId)>,
    ) -> Result<(), SyncError> {
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(TEMPLATES_TABLE)?;
            let mut oids_table = write_txn.open_table(TEMPLATES_OIDS_TABLE)?;

            for name in global_diff.deletes {
                let _ = table.remove(name.as_str());
                let _ = oids_table.remove(name.as_str());
            }

            for (namespaced_name, oid, source_name) in global_diff.upserts {
                let oid_str = oid.to_string();
                if let Ok(Some(existing_oid)) = oids_table.get(namespaced_name.as_str())
                    && existing_oid.value() == oid_str
                {
                    continue;
                }

                let updater = updaters
                    .iter()
                    .find(|u| u.source.name == source_name)
                    .ok_or_else(|| {
                        SyncError::GitOp(format!("Updater not found for source: {}", source_name))
                    })?;

                let content = updater.fetch_blob(oid)?;
                table.insert(namespaced_name.as_str(), content.as_str())?;
                oids_table.insert(namespaced_name.as_str(), oid_str.as_str())?;
            }

            // Update Metadata
            let mut meta_table = write_txn.open_table(METADATA_TABLE)?;
            for (source_name, head_oid) in updated_sources {
                let key = format!("last_commit_hash:{}", source_name);
                meta_table.insert(key.as_str(), head_oid.to_string().as_bytes())?;
            }

            // Generate FST
            let mut names: Vec<String> = table
                .iter()?
                .filter_map(|entry| {
                    let (name, _) = entry.ok()?;
                    Some(name.value().to_string())
                })
                .collect();

            names.sort();
            let mut builder = SetBuilder::memory();
            for name in names {
                builder
                    .insert(&name)
                    .map_err(|e| SyncError::Fst(e.to_string()))?;
            }
            fs::write(
                &self.config.fst_path,
                builder
                    .into_inner()
                    .map_err(|e| SyncError::Fst(e.to_string()))?,
            )?;
        }

        write_txn.commit()?;
        Ok(())
    }
}
