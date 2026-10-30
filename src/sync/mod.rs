use std::str::FromStr;

use gix::{ObjectId, Repository, bstr::BStr, traverse::tree::visit::Action};
use redb::{Database, ReadableDatabase, ReadableTable};

use crate::{
    Config, TemplateSource,
    cache::FileCached,
    error::SyncError,
    tables::{METADATA_TABLE, TEMPLATES_OIDS_TABLE, TEMPLATES_TABLE},
};

pub struct TemplateVisitor<'a> {
    source_name: &'a str,
    repo: &'a gix::Repository,
    upserts: &'a mut Vec<Changed>,
}

impl<'a> TemplateVisitor<'a> {
    pub fn new(
        source_name: &'a str,
        repo: &'a gix::Repository,
        upserts: &'a mut Vec<Changed>,
    ) -> Self {
        Self {
            source_name,
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
        if entry.filename.ends_with(b".gitignore") {
            // "macOS.gitignore" -> "macos" (drops any parent folder)
            let stem = entry
                .filename
                .strip_suffix(b".gitignore")
                .unwrap_or(entry.filename);
            let name = String::from_utf8_lossy(stem).to_lowercase();
            let namespace = TemplateSource::key_of(self.source_name, &name);

            // Fetch blob and stream directly into `upserts`!
            if let Ok(blob) = self.repo.find_object(entry.oid).map(|o| o.into_blob()) {
                let content = String::from_utf8_lossy(&blob.data).to_string();

                self.upserts.push(Changed {
                    namespace,
                    oid: entry.oid.to_owned(),
                    content,
                });
            }
        }

        Action::Continue(false)
    }
}

pub fn ensure_fail(source: &TemplateSource, force: bool) -> Repository {
    // TODO: this need to be success
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
                .and_then(|h| h.referent_name().map(|r| r.to_string()))
                .or_else(|| repo.find_reference("refs/heads/main").ok().map(|r| r.name().to_string()))
                .or_else(|| repo.find_reference("refs/heads/master").ok().map(|r| r.name().to_string()))
                .expect("fails to resolve default branch");

            let branch = source.branch.as_deref().unwrap_or(&default_branch);
            let refspec = format!("+{branch}:{branch}");

            let remote = repo
                .find_remote("origin")
                .expect("fails to find remote origin");

            let remote = remote
                .with_refspecs(
                    [refspec.as_str()],
                    gix::remote::Direction::Fetch,
                )
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

fn traverse_full(repo: &Repository, oid: ObjectId, source_name: &str, upserts: &mut Vec<Changed>) {
    // populate the upserts since we don't have it yet in redb for this specific source
    let head = repo
        .find_object(oid)
        .expect("fails to fetch object head for this tree");

    let tree = head.peel_to_tree().expect("fails to fetch the tree");
    let mut visitor = TemplateVisitor::new(source_name, repo, upserts);

    tree.traverse()
        .breadthfirst(&mut visitor)
        .expect("fail to populate upserts");
}

fn handle_change(
    change: gix::object::tree::diff::Change,
    source: &str,
    repo: &Repository,
    upserts: &mut Vec<Changed>,
    deletes: &mut Vec<String>,
) {
    use gix::object::tree::diff::Change;

    let (location, oid) = match change {
        Change::Addition { location, id, .. }
        | Change::Modification { location, id, .. }
        | Change::Rewrite { location, id, .. } => (location, Some(id.detach())),
        Change::Deletion { location, .. } => (location, None),
    };

    if location.ends_with(b".gitignore") {
        let filename = location.rsplit(|&b| b == b'/').next().unwrap_or(location);
        let stem = filename.strip_suffix(b".gitignore").unwrap_or(filename);
        let name = String::from_utf8_lossy(stem).to_lowercase();
        let namespace = TemplateSource::key_of(source, &name);

        match oid {
            Some(oid) => {
                let blob = repo
                    .find_object(oid)
                    .expect("fails to fetch object_id")
                    .into_blob();

                let content = String::from_utf8_lossy(&blob.data).to_string();

                upserts.push(Changed {
                    namespace,
                    oid,
                    content,
                });
            }
            _ => {
                deletes.push(namespace);
            }
        }
    }
}

// fetch current registered source from the config
//
pub fn sync(config: &Config, force: bool) -> Result<(), SyncError> {
    let db = match Database::open(&config.db_path) {
        Ok(db) => db,
        Err(_) => {
            if let Some(parent) = config.db_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            Database::create(&config.db_path).expect("fails to open db")
        }
    };

    let mut has_changes = false;

    for source in &config.sources {
        // TODO: better to do retry for this
        let repo = ensure_fail(source, force);

        let new_oid = repo
            .head()
            .ok()
            .and_then(|head| head.id())
            .map(|id| id.detach())
            .expect("fails to fetch oid from local repo");

        // this is basically contains per source last version
        let key = source.version_key();

        let mut upserts: Vec<Changed> = vec![];
        let mut deletes: Vec<String> = vec![];

        // now let's compare it with TEMPLATES_OIDS_TABLE for each source
        // old_oid <- TEMPLATES_OIDS_TABLE
        // new_oid <- repo.head_oid
        //

        {
            // XXX(@zerosign): stupid RAAI tricks for drop ReadTransaction :')

            let rtx = db.begin_read().expect("fails to read only");

            let oids = rtx
                .open_table(METADATA_TABLE)
                .expect("fails to open oids tables");

            let version = (!force)
                .then(|| oids.get(key.as_str()).expect("fails to fetch value"))
                .flatten();

            match version {
                Some(value) => {
                    let raw = str::from_utf8(value.value())
                        .expect("fail to fetch data from metadata table");

                    let old_oid = ObjectId::from_str(raw)
                        .expect("fails to initialize object id from redb Oids tables");

                    if new_oid.eq(&old_oid) {
                        // the oid are already same then let's do nothing then
                        // (None, false)
                        continue;
                    }

                    // the new oid aren't equal to old oid
                    // we need to traverse the walktree to update
                    //
                    let old_tree = repo
                        .find_object(old_oid)
                        .ok()
                        .and_then(|obj| obj.peel_to_tree().ok());

                    match old_tree {
                        Some(old_tree) => {
                            let new_tree = repo
                                .find_object(new_oid)
                                .expect("fail to find the object_id")
                                .peel_to_tree()
                                .expect("fails to worktree");

                            old_tree
                                .changes()
                                .expect("fails to fetch changes")
                                .for_each_to_obtain_tree(&new_tree, |change| {
                                    handle_change(
                                        change,
                                        &source.name,
                                        &repo,
                                        &mut upserts,
                                        &mut deletes,
                                    );

                                    Ok::<gix::diff::tree::visit::Action, SyncError>(
                                        gix::diff::tree::visit::Action::Continue(()),
                                    )
                                })
                                .expect("fails to populate diff");
                        }
                        _ => {
                            // Fallback if old_oid commit is missing from repo: full traversal
                            traverse_full(&repo, new_oid, &source.name, &mut upserts);
                        }
                    }
                }
                _ => {
                    // value is not exists yet in the database
                    // we need to populate and traverse the walktree to update

                    // populate the upserts since we don't have it yet in redb for this specific source
                    traverse_full(&repo, new_oid, &source.name, &mut upserts);
                }
            };
        }
        // We know have every changes that need to be populated in upserts & deletes

        has_changes = has_changes || !deletes.is_empty() || !upserts.is_empty();

        let wtx = db.begin_write().expect("fails to do write tx");

        {
            // XXX(@zerosign): another RAAI hack

            let mut tmpls = wtx
                .open_table(TEMPLATES_TABLE)
                .expect("fails to open template table for write");

            let mut toids = wtx
                .open_table(TEMPLATES_OIDS_TABLE)
                .expect("fails to template oids for write");

            // delete all keys that need to be deleted first
            for item in deletes {
                tmpls
                    .remove(item.as_str())
                    .expect("fails to delete tmpl content");

                toids
                    .remove(item.as_str())
                    .expect("fails to delete tmpl oids");
            }

            // upserts all keys that need to updated
            for item in upserts {
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
                .insert(key.clone().as_str(), version.as_bytes())
                .expect("fails to update version of source");
        }

        wtx.commit().expect("fails to delete");
    }

    let reindex = has_changes || !config.cache_path.exists();

    if !reindex {
        return Ok(());
    }

    let rtx = db.begin_read().expect("fails to read only");

    let tmpls = rtx
        .open_table(TEMPLATES_TABLE)
        .expect("fails to open template table for read");

    let mut names: Vec<String> = tmpls
        .iter()
        .expect("fails to scan keys from template table")
        .flatten()
        .map(|(k, _)| {
            let val = k.value();
            val.split_once("/")
                .map_or(val, |(_, name)| name)
                .to_string()
        })
        .collect();

    names.sort_unstable();
    names.dedup();

    // TODO(@zerosign) update the index/cache for this source, we want to use rkyv for this
    // this will be implemented using ../cache.rs
    FileCached::save(&config.cache_path, names).expect("fails to save template names cache");

    Ok(())
}
