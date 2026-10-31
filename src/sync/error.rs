use thiserror::Error;

use crate::storage::StorageError;

#[derive(Error, Debug)]
pub enum SyncError {
    #[error("Git open error: {0}")]
    Open(#[from] Box<gix::open::Error>),
    #[error("Git clone failed: {0}")]
    Clone(String),
    #[error("Git fetch failed: {0}")]
    Fetch(String),
    #[error("Git head error: {0}")]
    Head(#[from] gix::reference::head_id::Error),
    #[error("Git ref error: {0}")]
    RefFind(#[from] gix::reference::find::existing::Error),
    #[error("Git object error: {0}")]
    Object(#[from] gix::object::find::existing::Error),
    #[error("Git tree peel error: {0}")]
    Peel(#[from] gix::object::peel::to_kind::Error),
    #[error("Git tree diff error: {0}")]
    Diff(#[from] Box<gix::object::tree::diff::for_each::Error>),
    #[error("Git diff options error: {0}")]
    DiffOptions(#[from] Box<gix::diff::options::init::Error>),
    #[error("Git operation error: {0}")]
    Op(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<gix::open::Error> for SyncError {
    fn from(e: gix::open::Error) -> Self {
        Self::Open(Box::new(e))
    }
}

impl From<gix::object::tree::diff::for_each::Error> for SyncError {
    fn from(e: gix::object::tree::diff::for_each::Error) -> Self {
        Self::Diff(Box::new(e))
    }
}

impl From<gix::diff::options::init::Error> for SyncError {
    fn from(e: gix::diff::options::init::Error) -> Self {
        Self::DiffOptions(Box::new(e))
    }
}
