use std::str::FromStr;

use gix::{ObjectId, Repository, bstr::BStr, traverse::tree::visit::Action};

use crate::{
    TemplateSource,
    error::SyncError,
    sync::{Changed, Changes},
};

pub(crate) struct TemplateVisitor<'a> {
    source: &'a str,
    repo: &'a gix::Repository,
    upserts: &'a mut Vec<Changed>,
}

impl<'a> TemplateVisitor<'a> {
    pub fn new(source: &'a str, repo: &'a gix::Repository, upserts: &'a mut Vec<Changed>) -> Self {
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

pub(crate) fn ensure_fail(source: &TemplateSource, force: bool) -> Repository {
    let path = &source.path;

    if force && path.exists() {
        let _ = std::fs::remove_dir_all(path);
    }

    if let Ok(repo) = gix::open(path) {
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
    } else {
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

pub struct SourceRepository {
    source: String,
    repo: gix::Repository,
}

impl SourceRepository {
    pub fn open_or_fetch(source: &TemplateSource, force: bool) -> Result<Self, SyncError> {
        let repo = ensure_fail(source, force);

        Ok(Self {
            source: source.name.clone(),
            repo,
        })
    }

    /// Returns the current commit hash as a plain String
    pub fn current(&self) -> Result<String, SyncError> {
        let oid = self
            .repo
            .head()?
            .id()
            .expect("fails to fetch current oid")
            .detach()
            .to_string();

        Ok(oid)
    }

    fn traverse_full(&self, oid: ObjectId, changes: &mut Changes) {
        // populate the upserts since we don't have it yet in redb for this specific source
        let head = self
            .repo
            .find_object(oid)
            .expect("fails to fetch object head for this tree");

        let tree = head.peel_to_tree().expect("fails to fetch the tree");
        let mut visitor =
            TemplateVisitor::new(self.source.as_str(), &self.repo, &mut changes.upserts);

        tree.traverse()
            .breadthfirst(&mut visitor)
            .expect("fail to populate upserts");
    }

    fn handle_change(&self, change: gix::object::tree::diff::Change, changes: &mut Changes) {
        use gix::object::tree::diff::Change;

        let (location, oid) = match change {
            Change::Addition { location, id, .. }
            | Change::Modification { location, id, .. }
            | Change::Rewrite { location, id, .. } => (location, Some(id.detach())),
            Change::Deletion { location, .. } => (location, None),
        };

        let Some(namespace) = TemplateSource::key_of_loc(self.source.as_str(), location) else {
            return;
        };

        match oid {
            Some(oid) => {
                let blob = self
                    .repo
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

    pub fn changes_between(
        &self,
        old_oid: Option<String>,
        new_oid: &str,
    ) -> Result<Changes, SyncError> {
        let mut changes = Changes::default();

        let new_oid = ObjectId::from_str(&new_oid).expect("fails to convert to objectid");
        let old_oid = old_oid.and_then(|s| ObjectId::from_str(&s).ok());

        let old_tree = old_oid
            .and_then(|oid| self.repo.find_object(oid).ok())
            .and_then(|obj| obj.peel_to_tree().ok());

        match old_tree {
            Some(old_tree) => {
                let new_tree = self
                    .repo
                    .find_object(new_oid)
                    .expect("fails to find object")
                    .peel_to_tree()
                    .expect("fails to pin to a tree");

                old_tree
                    .changes()
                    .expect("fails to get the changes from old tree")
                    .for_each_to_obtain_tree(&new_tree, |change| {
                        self.handle_change(change, &mut changes);
                        Ok::<_, SyncError>(gix::diff::tree::visit::Action::Continue(()))
                    })
                    .expect("fails to traverse");
            }
            None => {
                self.traverse_full(new_oid, &mut changes);
            }
        }

        Ok(changes)
    }
}
