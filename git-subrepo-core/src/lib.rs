pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod error;
pub mod git_cli;
pub mod gitrepo;
pub mod refs;
pub mod remote;
pub mod repo;
pub mod subdir;

pub mod plumbing;

pub use error::{Error, Result, SubrepoResultExt};
pub use gitrepo::{GitRepoState, JoinMethod};
pub use refs::SubrepoRefs;
