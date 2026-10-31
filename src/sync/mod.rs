use std::{path::Path, str::FromStr};

use gix::{ObjectId, Repository, bstr::BStr, traverse::tree::visit::Action};
use redb::{Database, ReadableDatabase, ReadableTable};

use crate::{
    Config, TemplateSource,
    cache::FileCached,
    error::SyncError,
    tables::{METADATA_TABLE, TEMPLATES_OIDS_TABLE, TEMPLATES_TABLE},
};

pub struct TemplateVisitor<'a> {
    source: &'a str,
    repo: &'a gix::Repository,
    upserts: &'a mut Vec<Changed>,
}

impl<'a> TemplateVisitor<'a> {
    pub fn new(
        source: &'a str,
        repo: &'a gix::Repository,
        upserts: &'a mut Vec<Changed>,
    ) -> Self {
        Self {
            source,
            repo,
            upserts,
        }
    }
}

impl gix::traverse::tree::Visit for TemplateVisitor<'_> {
    fn pop_back_tracked_path_and_set_current(&mut self) {}

    // 1. When entering a queued directory, restore its parent path
    fn pop_front_tracked_path_and_set_current(&mut self) {}

    // 2. Queue directory paths for breadthfirst descent
    fn push_back_tracked_path_component(&mut self, _component: &BStr) {}

    fn push_path_component(&mut self, _component: &BStr) {}

    fn pop_path_component(&mut self) {}

    fn visit_tree(&mut self, _entry: &gix::objs::tree::EntryRef<'_>) -> Action {
        Action::Continue(true) // Recurse into directories
    }

    // 3. For files: filter on the fly!
    fn visit_nontree(&mut self, entry: &gix::objs::tree::EntryRef<'_>) -> Action {
        let Some(namespace) = TemplateSource::key_of_loc(self.source, entry.filename) else {
            return Action::Continue(false);
        };

        let Ok(blob) = self.repo.find_object(entry.oid).map(gix::Object::into_blob) else {
            return Action::Continue(false);
        };

        let content = String::from_utf8_lossy(&blob.data).to_string();

        self.upserts.push(Changed {
            namespace,
            oid: entry.oid.to_owned(),
            content,
        });

        Action::Continue(false)
    }
}

pub fn ensure_fail(source: &TemplateSource, force: bool) -> Repository {
    let path = &source.path;

    if force && path.exists() {
        let _ = std::fs::remove_dir_all(path);
    }

    match gix::open(path) {
        Ok(repo) => {
            let packed_lock = path.join("packed-refs.lock");

            if packed_lock.exists() {
                let _ = std::fs::remove_file(packed_lock);
            }

            // Fetch latest commits from origin into local branches
            let default_branch = repo
                .head()
                .ok()
                .and_then(|h| h.referent_name().map(std::string::ToString::to_string))
                .or_else(|| {
                    repo.find_reference("refs/heads/main")
                        .ok()
                        .map(|r| r.name().to_string())
                })
                .or_else(|| {
                    repo.find_reference("refs/heads/master")
                        .ok()
                        .map(|r| r.name().to_string())
                })
                .expect("fails to resolve default branch");

            let branch = source.branch.as_deref().unwrap_or(&default_branch);
            let refspec = format!("+{branch}:{branch}");

            let remote = repo
                .find_remote("origin")
                .expect("fails to find remote origin");

            let remote = remote
                .with_refspecs([refspec.as_str()], gix::remote::Direction::Fetch)
                .expect("fails to set refspecs");

            let conn = remote
                .connect(gix::remote::Direction::Fetch)
                .expect("fail to connect to remote");

            let prep = conn
                .prepare_fetch(&mut gix::progress::Discard, Default::default())
                .expect("fails to fetch");

            let _ = prep
                .receive(&mut gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
                .expect("fails to receive fetches");

            repo
        }
        _ => {
            let parent = path.parent().expect("fails to fetch parent dir");
            std::fs::create_dir_all(parent).expect("fails to create parent dir");

            let mut op = gix::prepare_clone_bare(source.url.as_str(), path)
                .expect("fails to clone the bare repo");

            // https://git-scm.com/book/en/v2/Git-Internals-The-Refspec
            //
            // so this means every fetch will update respected local index
            // this should be safe since we don't do update from local repo
            //
            // This simplify a lot of things, so technically after this we can assume
            // all local oid in the repo that being tracked already on the newest oid.
            //
            let (repo, _) = op
                .fetch_only(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
                .expect("fail to fetch the repo");

            repo
        }
    }
}

pub struct Changed {
    pub namespace: String,
    pub oid: ObjectId,
    pub content: String,
}

#[derive(Default)]
pub struct Changes {
    pub upserts: Vec<Changed>,
    pub deletes: Vec<String>,
}

impl Changes {
    pub const fn is_empty(&self) -> bool {
        self.upserts.is_empty() && self.deletes.is_empty()
    }
}

fn traverse_full(repo: &Repository, oid: ObjectId, source: &str, changes: &mut Changes) {
    // populate the upserts since we don't have it yet in redb for this specific source
    let head = repo
        .find_object(oid)
        .expect("fails to fetch object head for this tree");

    let tree = head.peel_to_tree().expect("fails to fetch the tree");
    let mut visitor = TemplateVisitor::new(source, repo, &mut changes.upserts);

    tree.traverse()
        .breadthfirst(&mut visitor)
        .expect("fail to populate upserts");
}

fn traverse_diff(
    repo: &Repository,
    source: &str,
    old_oid: ObjectId,
    new_oid: ObjectId,
    changes: &mut Changes,
) {
    let old_tree = repo
        .find_object(old_oid)
        .ok()
        .and_then(|obj| obj.peel_to_tree().ok());

    match old_tree {
        Some(old_tree) => {
            let new_tree = repo
                .find_object(new_oid)
                .expect("fail to find object_id")
                .peel_to_tree()
                .expect("fails to worktree");

            old_tree
                .changes()
                .expect("fails to fetch changes")
                .for_each_to_obtain_tree(&new_tree, |change| {
                    handle_change(change, source, repo, changes);
                    Ok::<_, SyncError>(gix::diff::tree::visit::Action::Continue(()))
                })
                .expect("fails to populate diff");
        }
        None => {
            traverse_full(repo, new_oid, source, changes);
        }
    }
}

fn handle_change(
    change: gix::object::tree::diff::Change,
    source: &str,
    repo: &Repository,
    changes: &mut Changes,
) {
    use gix::object::tree::diff::Change;

    let (location, oid) = match change {
        Change::Addition { location, id, .. }
        | Change::Modification { location, id, .. }
        | Change::Rewrite { location, id, .. } => (location, Some(id.detach())),
        Change::Deletion { location, .. } => (location, None),
    };

    let Some(namespace) = TemplateSource::key_of_loc(source, location) else {
        return;
    };

    match oid {
        Some(oid) => {
            let blob = repo
                .find_object(oid)
                .expect("fails to fetch object_id")
                .into_blob();

            let content = String::from_utf8_lossy(&blob.data).to_string();

            changes.upserts.push(Changed {
                namespace,
                oid,
                content,
            });
        }
        _ => {
            changes.deletes.push(namespace);
        }
    }
}

fn apply_changes(db: &redb::Database, vkey: &str, new_oid: ObjectId, changes: Changes) {
    let wtx = db.begin_write().expect("fails to do write tx");

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
            .open_table(METADATA_TABLE)
            .expect("fails to open versions table for writes");

        let version = new_oid.to_string();

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

fn rebuild_cache<P>(db: &redb::Database, path: P) -> Result<(), SyncError>
where
    P: AsRef<Path>,
{
    let rtx = db.begin_read().expect("fails to read only");

    // this contains source/template => template content
    let tmpls = rtx
        .open_table(TEMPLATES_TABLE)
        .expect("fails to open template table for read");

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

    // TODO(@zerosign) update the index/cache for this source, we want to use rkyv for this
    // this will be implemented using ../cache.rs
    FileCached::save(&path, names).expect("fails to save template names cache");

    Ok(())
}

// fetch current registered source from the config
//
pub fn sync(config: &Config, force: bool) -> Result<(), SyncError> {
    let db = Database::open(&config.db_path).or_else(|_| {
        let _ = config.db_path.parent().map(std::fs::create_dir_all);
        Database::create(&config.db_path)
    })?;

    let mut has_changes = false;

    for source in &config.sources {
        // TODO: better to do retry for this
        let repo = ensure_fail(source, force);

        let new_oid = repo
            .head()
            .ok()
            .and_then(|head| head.id())
            .map(gix::Id::detach)
            .expect("fails to fetch oid from local repo");

        // this is basically contains per source last version
        let vkey = source.version_key();

        let mut changes = Changes::default();

        // now let's compare it with TEMPLATES_OIDS_TABLE for each source
        // old_oid <- TEMPLATES_OIDS_TABLE
        // new_oid <- repo.head_oid
        //

        {
            // XXX(@zerosign): stupid RAAI tricks for drop ReadTransaction :')

            let old_oid: Option<ObjectId> = (!force)
                .then(|| {
                    let rtx = db.begin_read().expect("fails to read only");

                    // current version tables (this contains current source version)
                    let versions = rtx
                        .open_table(METADATA_TABLE)
                        .expect("fails to open versions tables");

                    versions.get(vkey.as_str()).ok().flatten().and_then(|v| {
                        str::from_utf8(v.value())
                            .ok()
                            .and_then(|s| ObjectId::from_str(s).ok())
                    })
                })
                .flatten();

            match old_oid {
                // the oid aren't same
                Some(old_oid) if old_oid != new_oid => {
                    traverse_diff(&repo, &source.name, old_oid, new_oid, &mut changes);
                }
                // the oid are already same then let's do nothing then
                Some(_) => continue,
                _ => {
                    // value is not exists yet in the database
                    // we need to populate and traverse the walktree to update

                    // populate the upserts since we don't have it yet in redb for this specific source
                    traverse_full(&repo, new_oid, &source.name, &mut changes);
                }
            }
        }
        // We know have every changes that need to be populated in upserts & deletes

        has_changes = has_changes || !changes.is_empty();

        apply_changes(&db, &vkey, new_oid, changes);
    }

    let reindex = has_changes || !config.cache_path.exists();

    if !reindex {
        return Ok(());
    }

    rebuild_cache(&db, &config.cache_path).expect("fails to rebuild cache");

    Ok(())
}
