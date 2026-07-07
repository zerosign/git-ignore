use std::path::Path;

use crate::{
    TemplateSource,
    error::SyncError,
    result::Result,
    sync::RepoStatus,
};

#[derive(Debug)]
pub enum RepoState {
    Valid,
    Missing,
    Corrupted,
    NeedsFetch,
    ForceReclone,
}

impl RepoState {
    #[must_use]
    pub fn determine(path: &Path, update: bool, force: bool) -> Self {
        match (path.exists(), force, update, gix::open(path).is_err()) {
            (false, _, _, _) => RepoState::Missing,
            (_, _, _, true) => RepoState::Corrupted,
            (_, true, _, _) => RepoState::ForceReclone,
            (_, _, true, false) => RepoState::NeedsFetch,
            _ => RepoState::Valid,
        }
    }

    /// Ensures the template repository is valid and updated.
    ///
    /// # Errors
    /// Returns `SyncError` if cloning, fetching, or checking out references fails.
    pub fn ensure(
        source: &TemplateSource,
        update: bool,
        force: bool,
    ) -> Result<RepoStatus, SyncError> {
        let path = &source.path;

        match RepoState::determine(path, update, force) {
            RepoState::Valid => return Ok(RepoStatus::Matched),
            RepoState::Missing => println!(
                "Cloning gitignore templates repository [{}]...",
                source.name
            ),
            RepoState::NeedsFetch => {
                println!(
                    "Fetching updates for gitignore templates repository [{}]...",
                    source.name
                );

                let repo = gix::open(path)?;

                let mut old_oid = None;
                if let Ok(head) = repo.head()
                    && let Some(id) = head.id()
                {
                    old_oid = Some(id.detach());
                }

                let remote = repo.find_remote("origin").map_err(|e| {
                    SyncError::GitOp(format!("Could not find remote 'origin': {e}"))
                })?;

                let connection = remote
                    .connect(gix::remote::Direction::Fetch)
                    .map_err(|e| SyncError::GitOp(format!("Connection failed: {e}")))?;

                let _outcome = connection
                    .prepare_fetch(
                        &mut gix::progress::Discard,
                        gix::remote::ref_map::Options::default(),
                    )
                    .map_err(|e| SyncError::GitOp(format!("Prepare fetch failed: {e}")))?
                    .receive(&mut gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
                    .map_err(|e| SyncError::GitOp(format!("Receive failed: {e}")))?;

                let mut new_oid = None;

                if let Ok(head) = repo.head()
                    && let Some(referent) = head.referent_name()
                    && let Some(short_name) = referent.as_bstr().strip_prefix(b"refs/heads/")
                {
                    let remote_ref_name = format!(
                        "refs/remotes/origin/{}",
                        String::from_utf8_lossy(short_name)
                    );

                    if let Ok(remote_ref) = repo.find_reference(&remote_ref_name) {
                        let remote_oid = remote_ref.id();
                        new_oid = Some(remote_oid.detach());

                        let edit = gix::refs::transaction::RefEdit {
                            change: gix::refs::transaction::Change::Update {
                                log: gix::refs::transaction::LogChange {
                                    mode: gix::refs::transaction::RefLog::AndReference,
                                    force_create_reflog: false,
                                    message: "update local branch to match remote tracking branch"
                                        .into(),
                                },
                                expected: gix::refs::transaction::PreviousValue::Any,
                                new: gix::refs::Target::Object(remote_oid.detach()),
                            },
                            name: referent.to_owned(),
                            deref: true,
                        };

                        repo.edit_reference(edit)
                            .map_err(|e| SyncError::GitOp(format!("Edit reference failed: {e}")))?;
                    }
                }

                if let (Some(old), Some(new)) = (old_oid, new_oid)
                    && old != new
                {
                    return Ok(RepoStatus::Updated {
                        old_oid: old,
                        new_oid: new,
                    });
                }

                return Ok(RepoStatus::Matched);
            }
            RepoState::Corrupted | RepoState::ForceReclone => {
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

        Ok(RepoStatus::Recloned)
    }
}
