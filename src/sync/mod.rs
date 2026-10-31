use gix::ObjectId;

use crate::{Config, storage::TemplateStorage, sync::repo::SourceRepository};

mod error;
mod repo;

pub use error::SyncError;

pub struct Changed {
    pub(crate) namespace: String,
    pub(crate) oid: ObjectId,
    pub(crate) content: String,
}

#[derive(Default)]
pub struct Changes {
    pub(crate) upserts: Vec<Changed>,
    pub(crate) deletes: Vec<String>,
}

impl Changes {
    const fn is_empty(&self) -> bool {
        self.upserts.is_empty() && self.deletes.is_empty()
    }
}

pub fn sync(storage: &TemplateStorage, config: &Config, force: bool) -> Result<(), SyncError> {
    let mut has_changes = false;

    for source in &config.sources {
        let srepo = SourceRepository::open_or_fetch(source, force)?;
        let new_oid = srepo.current()?;
        let vkey = source.version_key();

        let old_oid: Option<String> = (!force).then(|| storage.last_version_for(&vkey)).flatten();

        if old_oid.as_ref() == Some(&new_oid) {
            continue;
        }

        let changes = srepo.changes_between(old_oid, &new_oid)?;

        has_changes = has_changes || !changes.is_empty();

        storage.save_changes(&vkey, &new_oid, changes)?;
    }

    let reindex = has_changes || !config.cache_path.exists();

    if !reindex {
        return Ok(());
    }

    storage.build_cache()?;

    Ok(())
}
