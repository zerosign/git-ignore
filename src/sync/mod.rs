use gix::ObjectId;

use crate::{Config, error::SyncError, storage::TemplateStorage, sync::repo::SourceRepository};

mod repo;

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

//
pub fn sync(storage: &TemplateStorage, config: &Config, force: bool) -> Result<(), SyncError> {
    let mut has_changes = false;

    for source in &config.sources {
        let srepo = SourceRepository::open_or_fetch(source, force).expect("can't fetch repo");
        let new_oid = srepo.current().expect("can't fetch new oid");
        // let version = new_oid;
        let vkey = source.version_key();

        let old_oid: Option<String> = (!force).then(|| storage.last_version_for(&vkey)).flatten();

        if old_oid.as_ref() == Some(&new_oid) {
            continue;
        }

        let changes = srepo
            .changes_between(old_oid, &new_oid)
            .expect("fails to get changes between old and new id");

        // We know have every changes that need to be populated in upserts & deletes
        has_changes = has_changes || !changes.is_empty();

        storage.save_changes(&vkey, &new_oid, changes);
    }

    let reindex = has_changes || !config.cache_path.exists();

    if !reindex {
        return Ok(());
    }

    storage.build_cache().expect("fails to rebuild cache");

    Ok(())
}
