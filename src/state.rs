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
        match (path.exists(), update, gix::open(path).is_err()) {
            (false, _, _) => RepoState::Missing,
            (_, true, _) => RepoState::UpdateRequested,
            (_, _, true) => RepoState::Corrupted,
            _ => RepoState::Valid,
        }
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
        let key = format!("last_commit_hash:{}", source_name);

        let last_hash = read_txn
            .open_table(METADATA_TABLE)
            .ok()
            .and_then(|t| t.get(key.as_str()).ok().flatten())
            .map(|b| String::from_utf8_lossy(b.value()).to_string());

        let state = match last_hash {
            Some(hash)
                if hash == head_oid.to_string() && is_source_populated(&read_txn, source_name) =>
            {
                SyncState::UpToDate
            }
            Some(hash) => gix::ObjectId::from_hex(hash.as_bytes())
                .map(SyncState::Incremental)
                .unwrap_or(SyncState::NeedsRebuild),
            None => SyncState::NeedsRebuild,
        };

        Ok(state)
    }
}

fn is_source_populated(txn: &redb::ReadTransaction, source_name: &str) -> bool {
    let prefix = format!("{}/", source_name);
    txn.open_table(TEMPLATES_TABLE).is_ok_and(|t| {
        t.range(prefix.as_str()..).is_ok_and(|mut r| {
            r.next()
                .and_then(|res| res.ok())
                .is_some_and(|(k, _)| k.value().starts_with(&prefix))
        })
    })
}
