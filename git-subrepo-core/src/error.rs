use std::error::Error as StdError;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

pub type DynError = Box<dyn StdError + Send + Sync + 'static>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    User(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Internal(#[from] DynError),
}

impl Error {
    pub fn user(message: impl Into<String>) -> Self {
        Error::User(message.into())
    }

    pub fn internal(err: impl StdError + Send + Sync + 'static) -> Self {
        Error::Internal(Box::new(err))
    }
}

pub trait SubrepoResultExt<T> {
    fn into_subrepo_result(self) -> Result<T>;
}

impl<T, E> SubrepoResultExt<T> for std::result::Result<T, E>
where
    E: StdError + Send + Sync + 'static,
{
    fn into_subrepo_result(self) -> Result<T> {
        self.map_err(|err| Error::internal(err))
    }
}
