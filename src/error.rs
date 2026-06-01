use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Environment error: {0}")]
    Env(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Error, Debug)]
pub enum SyncError {
    #[error("Git error: {0}")]
    Git(#[from] Box<gix::open::Error>),
    #[error("Git clone failed: {0}")]
    GitClone(#[from] Box<gix::clone::Error>),
    #[error("Git fetch failed: {0}")]
    GitFetch(#[from] Box<gix::clone::fetch::Error>),
    #[error("Git head error: {0}")]
    GitHead(#[from] gix::reference::head_id::Error),
    #[error("Git reference find error: {0}")]
    GitRefFind(#[from] gix::reference::find::existing::Error),
    #[error("Git commit error: {0}")]
    GitCommit(#[from] gix::object::find::existing::Error),
    #[error("Git tree error: {0}")]
    GitTree(#[from] gix::object::peel::to_kind::Error),
    #[error("Git tree diff for-each error: {0}")]
    GitDiffForEach(#[from] Box<gix::object::tree::diff::for_each::Error>),
    #[error("Git diff options init error: {0}")]
    GitDiffOptions(#[from] Box<gix::diff::options::init::Error>),
    #[error("Git operation failed: {0}")]
    GitOp(String),
    #[error("Git diff error: {0}")]
    GitDiff(String),
    #[error("Database error: {0}")]
    Database(#[from] redb::DatabaseError),
    #[error("Database transaction error: {0}")]
    DatabaseTransaction(#[from] redb::TransactionError),
    #[error("Database table error: {0}")]
    DatabaseTable(#[from] redb::TableError),
    #[error("Database storage error: {0}")]
    DatabaseStorage(#[from] redb::StorageError),
    #[error("Database commit error: {0}")]
    DatabaseCommit(#[from] redb::CommitError),
    #[error("FST error: {0}")]
    Fst(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Discovery error: {0}")]
    Discovery(String),
}

impl From<gix::open::Error> for SyncError {
    fn from(e: gix::open::Error) -> Self {
        Self::Git(Box::new(e))
    }
}

impl From<gix::clone::Error> for SyncError {
    fn from(e: gix::clone::Error) -> Self {
        Self::GitClone(Box::new(e))
    }
}

impl From<gix::clone::fetch::Error> for SyncError {
    fn from(e: gix::clone::fetch::Error) -> Self {
        Self::GitFetch(Box::new(e))
    }
}

impl From<gix::object::tree::diff::for_each::Error> for SyncError {
    fn from(e: gix::object::tree::diff::for_each::Error) -> Self {
        Self::GitDiffForEach(Box::new(e))
    }
}

impl From<gix::diff::options::init::Error> for SyncError {
    fn from(e: gix::diff::options::init::Error) -> Self {
        Self::GitDiffOptions(Box::new(e))
    }
}

#[derive(Error, Debug)]
pub enum GenerateError {
    #[error("Discovery error: {0}")]
    Discovery(String),
    #[error("Database error: {0}")]
    Database(#[from] redb::DatabaseError),
    #[error("Database table error: {0}")]
    DatabaseTable(#[from] redb::TableError),
    #[error("Database transaction error: {0}")]
    DatabaseTransaction(#[from] redb::TransactionError),
    #[error("Database storage error: {0}")]
    DatabaseStorage(#[from] redb::StorageError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Error, Debug)]
pub enum ListError {
    #[error("Discovery error: {0}")]
    Discovery(String),
    #[error("FST error: {0}")]
    Fst(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Error, Debug)]
pub enum InfoError {
    #[error("Git error: {0}")]
    Git(#[from] Box<gix::open::Error>),
    #[error("Git head error: {0}")]
    GitHead(#[from] gix::reference::head_id::Error),
    #[error("Database error: {0}")]
    Database(#[from] redb::DatabaseError),
    #[error("Database table error: {0}")]
    DatabaseTable(#[from] redb::TableError),
    #[error("Database transaction error: {0}")]
    DatabaseTransaction(#[from] redb::TransactionError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<gix::open::Error> for InfoError {
    fn from(e: gix::open::Error) -> Self {
        Self::Git(Box::new(e))
    }
}

#[derive(Error, Debug)]
pub enum CompactError {
    #[error("Database error: {0}")]
    Database(#[from] redb::DatabaseError),
    #[error("Compaction error: {0}")]
    Compaction(#[from] redb::CompactionError),
    #[error("Database transaction error: {0}")]
    DatabaseTransaction(#[from] redb::TransactionError),
    #[error("Database storage error: {0}")]
    DatabaseStorage(#[from] redb::StorageError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Error, Debug)]
pub enum CliError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Sync(#[from] Box<SyncError>),
    #[error(transparent)]
    Generate(#[from] GenerateError),
    #[error(transparent)]
    List(#[from] ListError),
    #[error(transparent)]
    Info(#[from] InfoError),
    #[error(transparent)]
    Compact(#[from] CompactError),
    #[error("Discovery error: {0}")]
    Discovery(String),
    #[error("Git error: {0}")]
    Git(String),
    #[error("Database error: {0}")]
    Database(#[from] redb::DatabaseError),
    #[error("Database transaction error: {0}")]
    DatabaseTransaction(#[from] redb::TransactionError),
    #[error("Database table error: {0}")]
    DatabaseTable(#[from] redb::TableError),
    #[error("Database storage error: {0}")]
    DatabaseStorage(#[from] redb::StorageError),
    #[error("Database commit error: {0}")]
    DatabaseCommit(#[from] redb::CommitError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<SyncError> for CliError {
    fn from(e: SyncError) -> Self {
        Self::Sync(Box::new(e))
    }
}
