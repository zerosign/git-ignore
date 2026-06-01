use std::path::Path;

use crate::{
    TemplateSource,
    defs::{METADATA_TABLE, TEMPLATES_TABLE},
    error::SyncError,
    result::Result,
    visitor::TemplateVisitor,
};

#[derive(Debug)]
pub enum RepoState {
    Valid,
    Missing,
    Corrupted,
    UpdateRequested,
}

impl RepoState {
    pub fn determine(path: &Path, update: bool) -> Self {
        if !path.exists() {
            return RepoState::Missing;
        }
        if update {
            return RepoState::UpdateRequested;
        }
        if gix::open(path).is_err() {
            return RepoState::Corrupted;
        }
        RepoState::Valid
    }

    pub fn ensure(source: &TemplateSource, update: bool) -> Result<(), SyncError> {
        let path = &source.path;

        match RepoState::determine(path, update) {
            RepoState::Valid => return Ok(()),
            RepoState::Missing => println!(
                "Cloning gitignore templates repository [{}]...",
                source.name
            ),
            RepoState::UpdateRequested | RepoState::Corrupted => {
                println!(
                    "Refreshing gitignore templates repository [{}]...",
                    source.name
                );
                let _ = std::fs::remove_dir_all(path);
            }
        }

        std::fs::create_dir_all(
            path.parent()
                .ok_or_else(|| SyncError::GitOp("Invalid templates path".to_string()))?,
        )?;

        let mut prepare =
            gix::prepare_clone_bare(source.url.as_str(), path).map_err(SyncError::from)?;

        let (_checkout, _outcome) = prepare
            .fetch_only(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
            .map_err(SyncError::from)?;

        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub enum SyncState {
    UpToDate,
    NeedsRebuild,
    Incremental(gix::ObjectId),
}

impl SyncState {
    pub fn fetch_all_templates(
        repo: &gix::Repository,
        head_oid: &gix::ObjectId,
    ) -> Result<Vec<(String, gix::ObjectId)>, SyncError> {
        let head_obj = repo.find_object(*head_oid).map_err(SyncError::from)?;
        let tree = head_obj.peel_to_tree().map_err(SyncError::from)?;

        let mut visitor = TemplateVisitor::new();

        tree.traverse()
            .breadthfirst(&mut visitor)
            .map_err(|e| SyncError::GitOp(e.to_string()))?;

        Ok(visitor.output())
    }

    pub fn determine<D: redb::ReadableDatabase>(
        db: &D,
        source_name: &str,
        head_oid: &gix::ObjectId,
        fst_path: &Path,
    ) -> Result<SyncState, SyncError> {
        if !fst_path.exists() {
            return Ok(SyncState::NeedsRebuild);
        }

        let read_txn = db.begin_read()?;

        if let Ok(meta_table) = read_txn.open_table(METADATA_TABLE) {
            let key = format!("last_commit_hash:{}", source_name);
            let last_oid_bytes = meta_table.get(key.as_str())?;

            if let Some(bytes) = last_oid_bytes {
                let last_hash_str = String::from_utf8_lossy(bytes.value());
                if last_hash_str == head_oid.to_string() {
                    if let Ok(templates_table) = read_txn.open_table(TEMPLATES_TABLE) {
                        let prefix = format!("{}/", source_name);
                        let has_templates = templates_table
                            .range(prefix.as_str()..)?
                            .next()
                            .and_then(|r| r.ok())
                            .map(|(k, _)| k.value().starts_with(&prefix))
                            .unwrap_or(false);

                        if has_templates {
                            return Ok(SyncState::UpToDate);
                        }
                    }
                } else if let Ok(last_oid) = gix::ObjectId::from_hex(last_hash_str.as_bytes()) {
                    return Ok(SyncState::Incremental(last_oid));
                }
            }
        }
        Ok(SyncState::NeedsRebuild)
    }
}
