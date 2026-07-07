use std::fs::{self, remove_file};

use fst::SetBuilder;
use gix::{ObjectId, Repository};
use redb::{Database, ReadableDatabase, ReadableTable};

use crate::{
    TemplateSource,
    config::Config,
    defs::{METADATA_TABLE, TEMPLATES_OIDS_TABLE, TEMPLATES_TABLE},
    error::SyncError,
    result::Result,
    state::RepoState,
    visitor::TemplateVisitor,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoStatus {
    Recloned,
    Updated {
        old_oid: ObjectId,
        new_oid: ObjectId,
    },
    Matched,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    Incremental {
        old_oid: ObjectId,
        new_oid: ObjectId,
    },
    Rebuild,
}

/// A pending template update.
pub struct SyncUpsert {
    pub namespace: String,
    pub oid: String,
    pub content: String,
}

/// Operations to be executed inside the database transaction.
#[derive(Default)]
pub struct SyncDiff {
    pub upserts: Vec<SyncUpsert>,
    pub deletes: Vec<String>,
}

impl SyncDiff {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.upserts.is_empty() && self.deletes.is_empty()
    }
}

/// Responsible for computing the diff for a single `TemplateSource`.
pub struct SyncUpdater<'a> {
    source: &'a TemplateSource,
    repo: Repository,
}

impl<'a> SyncUpdater<'a> {
    /// Creates a new `SyncUpdater` for the given template source.
    ///
    /// # Errors
    /// Returns `SyncError` if the local repository cannot be opened.
    pub fn new(source: &'a TemplateSource) -> Result<Self, SyncError> {
        let repo = gix::open(&source.path)?;
        Ok(Self { source, repo })
    }

    /// Returns the HEAD OID for the repository.
    /// Returns the HEAD OID for the repository.
    ///
    /// # Errors
    /// Returns `SyncError` if resolving the repository HEAD fails.
    pub fn head_oid(&self) -> Result<ObjectId, SyncError> {
        Ok(self
            .repo
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

    /// Computes the incremental changes between `old_oid` and `new_oid` and appends to the diff.
    ///
    /// # Errors
    /// Returns `SyncError` if reading git trees or repository data fails.
    pub fn populate_diff(
        &self,
        diff: &mut SyncDiff,
        old_oid: ObjectId,
        new_oid: ObjectId,
    ) -> Result<(), SyncError> {
        let old_tree = self.repo.find_object(old_oid)?.peel_to_tree()?;
        let new_tree = self.repo.find_object(new_oid)?.peel_to_tree()?;

        let mut to_upsert_oids = Vec::new();

        old_tree
            .changes()?
            .for_each_to_obtain_tree(&new_tree, |change| {
                use gix::object::tree::diff::Change;

                let (location, oid_opt) = match change {
                    Change::Addition { location, id, .. }
                    | Change::Modification { location, id, .. }
                    | Change::Rewrite { location, id, .. } => (location, Some(id.detach())),
                    Change::Deletion { location, .. } => (location, None),
                };
                if location.ends_with(b".gitignore") {
                    let name = location
                        .strip_suffix(b".gitignore")
                        .ok_or_else(|| SyncError::GitOp("Invalid suffix".to_string()))?;

                    let name_str = String::from_utf8_lossy(name).to_lowercase();
                    let namespaced_name = format!("{}/{}", self.source.name, name_str);

                    if let Some(oid) = oid_opt {
                        to_upsert_oids.push((namespaced_name, oid));
                    } else {
                        diff.deletes.push(namespaced_name);
                    }
                }

                Ok::<gix::diff::tree::visit::Action, SyncError>(
                    gix::diff::tree::visit::Action::Continue(()),
                )
            })?;

        for (name, oid) in to_upsert_oids {
            diff.upserts.push(SyncUpsert {
                namespace: name,
                oid: oid.to_string(),
                content: self.fetch_blob(oid)?,
            });
        }

        Ok(())
    }

    /// Traverses the entire tree at HEAD and schedules all templates for insertion, clearing old entries.
    ///
    /// # Errors
    /// Returns `SyncError` if reading references, git trees, or database values fails.
    pub fn populate_all(&self, db: &Database, diff: &mut SyncDiff) -> Result<(), SyncError> {
        let head_oid = self.head_oid()?;

        // Query all existing database keys for this source and delete them
        if let Ok(read_txn) = db.begin_read()
            && let Ok(table) = read_txn.open_table(TEMPLATES_TABLE)
        {
            let prefix = format!("{}/", self.source.name);
            if let Ok(mut iter) = table.range(prefix.as_str()..) {
                while let Some(Ok((k, _))) = iter.next() {
                    let key = k.value();
                    if key.starts_with(&prefix) {
                        diff.deletes.push(key.to_string());
                    } else {
                        break;
                    }
                }
            }
        }

        let all = self.fetch_all_templates(&head_oid)?;

        for (name, oid) in all {
            let namespaced_name = format!("{}/{}", self.source.name, name);

            diff.upserts.push(SyncUpsert {
                namespace: namespaced_name,
                oid: oid.to_string(),
                content: self.fetch_blob(oid)?,
            });
        }

        Ok(())
    }

    /// Traverses the entire tree at HEAD to fetch all templates.
    fn fetch_all_templates(
        &self,
        head_oid: &ObjectId,
    ) -> Result<Vec<(String, ObjectId)>, SyncError> {
        let head_obj = self.repo.find_object(*head_oid).map_err(SyncError::from)?;
        let tree = head_obj.peel_to_tree().map_err(SyncError::from)?;

        let mut visitor = TemplateVisitor::new();

        tree.traverse()
            .breadthfirst(&mut visitor)
            .map_err(|e| SyncError::GitOp(e.to_string()))?;

        Ok(visitor.output())
    }

    /// Fetches a blob from the repository.
    ///
    /// # Errors
    /// Returns `SyncError` if reading the blob object fails.
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
    #[must_use]
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    /// Ensures all local repositories exist and are updated if requested.
    ///
    /// # Errors
    /// Returns `SyncError` if checking or updating the local repository status fails.
    pub fn ensure_repos(
        &self,
        update: bool,
        force: bool,
    ) -> Result<Vec<(String, RepoStatus)>, SyncError> {
        let mut status = Vec::new();

        for source in &self.config.sources {
            let state = RepoState::ensure(source, update, force)?;
            status.push((source.name.clone(), state));
        }

        Ok(status)
    }

    /// Executes the full database sync.
    ///
    /// # Errors
    /// Returns `SyncError` if opening the database, computing repository updates, or committing writes fails.
    pub fn sync(&self, outcomes: &[(String, RepoStatus)]) -> Result<(), SyncError> {
        // 1. Open Database
        let db = if let Ok(db) = Database::create(&self.config.db_path) {
            db
        } else {
            let _ = remove_file(&self.config.db_path);
            Database::create(&self.config.db_path)?
        };

        // 2. Compute Diffs
        // This accumulates database additions, updates, and deletions across all configured sources.
        let mut diff = SyncDiff::new();
        let mut updated_sources = Vec::new();

        for source in &self.config.sources {
            let outcome = outcomes
                .iter()
                .find(|(name, _)| name == &source.name)
                .map_or(&RepoStatus::Matched, |(_, o)| o);

            let updater = SyncUpdater::new(source)?;
            let head_oid = updater.head_oid()?;

            // Let's determine the SyncAction
            // 1. Read last_commit_hash for this source from metadata table
            let mut last_commit_hash = None;
            if let Ok(read_txn) = db.begin_read()
                && let Ok(meta_table) = read_txn.open_table(METADATA_TABLE)
            {
                let key = format!("last_commit_hash:{}", source.name);
                if let Ok(Some(val)) = meta_table.get(key.as_str()) {
                    let hash_str = String::from_utf8_lossy(val.value()).to_string();
                    if let Ok(oid) = gix::ObjectId::from_hex(hash_str.as_bytes()) {
                        last_commit_hash = Some(oid);
                    }
                }
            }

            // 2. Map RepoStatus and last_commit_hash to Option<SyncAction>
            let action = match (outcome, last_commit_hash) {
                (RepoStatus::Updated { old_oid, new_oid }, Some(last_oid))
                    if *old_oid == last_oid =>
                {
                    Some(SyncAction::Incremental {
                        old_oid: *old_oid,
                        new_oid: *new_oid,
                    })
                }
                (RepoStatus::Matched, Some(last_oid)) if head_oid == last_oid => {
                    // Check if templates table actually has some entries for this source
                    // (just in case index was deleted but metadata remained)
                    let mut has_entries = false;
                    if let Ok(read_txn) = db.begin_read()
                        && let Ok(templates_table) = read_txn.open_table(TEMPLATES_TABLE)
                    {
                        let prefix = format!("{}/", source.name);
                        if let Ok(mut range) = templates_table.range(prefix.as_str()..)
                            && let Some(Ok((k, _))) = range.next()
                            && k.value().starts_with(&prefix)
                        {
                            has_entries = true;
                        }
                    }
                    if has_entries && self.config.fst_path.exists() {
                        None
                    } else {
                        Some(SyncAction::Rebuild)
                    }
                }
                (RepoStatus::Recloned | RepoStatus::Updated { .. } | RepoStatus::Matched, _) => {
                    Some(SyncAction::Rebuild)
                }
            };

            // 3. Execute SyncAction
            match action {
                None => {}
                Some(SyncAction::Incremental { old_oid, new_oid }) => {
                    updater.populate_diff(&mut diff, old_oid, new_oid)?;
                    updated_sources.push((source.name.clone(), new_oid));
                }
                Some(SyncAction::Rebuild) => {
                    updater.populate_all(&db, &mut diff)?;
                    updated_sources.push((source.name.clone(), head_oid));
                }
            }
        }

        if diff.is_empty() && updated_sources.is_empty() && self.config.fst_path.exists() {
            return Ok(());
        }

        // 3. Apply Diff in a single transaction
        self.apply_updates(&db, diff, updated_sources)
    }

    /// Internal helper to apply computed diffs and update metadata/index.
    fn apply_updates(
        &self,
        db: &Database,
        diff: SyncDiff,
        updated_sources: Vec<(String, ObjectId)>,
    ) -> Result<(), SyncError> {
        let write_txn = db.begin_write()?;

        {
            let mut table = write_txn.open_table(TEMPLATES_TABLE)?;
            let mut oids_table = write_txn.open_table(TEMPLATES_OIDS_TABLE)?;

            for name in diff.deletes {
                let _ = table.remove(name.as_str());
                let _ = oids_table.remove(name.as_str());
            }

            for upsert in diff.upserts {
                let matched = oids_table
                    .get(upsert.namespace.as_str())
                    .is_ok_and(|opt| opt.is_some_and(|guard| guard.value() == upsert.oid));

                if matched {
                    continue;
                }

                table.insert(upsert.namespace.as_str(), upsert.content.as_str())?;
                oids_table.insert(upsert.namespace.as_str(), upsert.oid.as_str())?;
            }

            // Update Metadata
            let mut meta_table = write_txn.open_table(METADATA_TABLE)?;

            for (source_name, head_oid) in updated_sources {
                let key = format!("last_commit_hash:{source_name}");
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
